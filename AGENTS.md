# Project: fxrate — Offline Currency Converter CLI

A Rust offline currency conversion command-line tool with historical exchange-rate charts.

## Scope

This file documents the current behavior and project conventions.
`src/` is the implementation source of truth; `src/main.rs` is the entry point, with the code split into modules (`cli`, `current`, `history`, `provider`, `render`, `series`, `storage`).
Verify behavior against the source and tests when changing this document.

## Tech stack

- Rust with Cargo and standard library plus `reqwest`, `serde`, `serde_json`, `chrono`, `rusqlite` (bundled), `csv`, `zip`, `textplots`, `terminal_size`, and `owo-colors`
- Source: `src/`; manifest: `Cargo.toml`
- Build: `cargo build --release --locked`
- Test: `cargo test --locked` (unit tests in modules + `tests/chart.rs`)
- Cargo build artifact: `target/release/fxrate`; Cargo's `target/` directory is gitignored
- `rusqlite` is bundled and compiles SQLite's C sources, so a C compiler is required for the build

## Data source

Two rate providers, selected with `-p/--provider` or the `provider` config key: `frankfurter` (the default) and `exchange-api`.

- Frankfurter API v2: `GET https://api.frankfurter.dev/v2/rates?base=EUR`
- exchange-api (fawazahmed0/exchange-api): `GET https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/eur.min.json`, falling back to `https://latest.currency-api.pages.dev/v1/currencies/eur.min.json` if the primary endpoint fails

Frankfurter responds with a JSON **array** of rows: `[{"date","base","quote","rate"}, ...]`; the base currency itself is not in the list.
exchange-api responds with a single JSON object: `{"date", "eur": {"usd": ..., "jpy": ..., ...}}`; the base currency is included.
Available currencies and counts may change by provider.

Both quote rates as units of the target currency per 1 EUR, so the cached snapshot has the same shape either way.
The full snapshot is cached locally.
All conversions are computed offline from the base-EUR snapshot: `amount * rate[dst] / rate[src]` — no network request is needed while a compatible fresh cache is available.

### Historical rates (chart command)

- Source: ECB reference rates, full history CSV: `https://www.ecb.europa.eu/stats/eurofxref/eurofxref-hist.zip` (contains `eurofxref-hist.csv`; dates from 1999, values EUR-based, `N/A` for missing entries; old currency columns such as EEK/LTL are dynamic)
- The `fxrate chart` command plots from the ECB history, and `fxrate` (convert) reads the same ECB history when given `--date`.
  History is stored per provider in SQLite (`history.db`) and is never mixed with the `rates.json` snapshot
- Chart data split: history comes from `history.db` (ECB); the live tail — the point at the cached snapshot's
  `rates.json` date — comes from the rates cache so the chart's right edge matches the convert command.
  When that date is on the last ECB day the ECB point is replaced; when it is after (weekend/holiday) a point
  is appended and the default range extends to it. The snapshot date must be within 4 days of the last ECB day
  (a weekend plus one holiday); a larger gap means the history is stale and no splice happens. A snapshot dated
  before the last ECB day or a currency missing from the snapshot (warned on stderr) also disables the tail.
  The chart never fetches live rates: it reads `rates.json` as-is (refresh it by running the convert command).
- On first chart use (or when the requested `--from`/`--to` range is not covered, or with `-u/--update`) the full CSV is downloaded and upserted in a transaction (sync/fallback/error policy is under **Behavior**)
- `history_coverage` records successfully synced ranges so weekend-only ranges are not mistaken for missing downloads; no weekend/holiday data is fabricated and no interpolation is done
- Chart points are the per-date intersection of both currencies (both must have data that day), computed as `EUR→target / EUR→source`; EUR is a unity series over the trading-date universe
- `date` = the rates' business date from the API; `fetched_at` = local fetch time; both are stored in the cache
- The cache records the provider that produced it.
  A provider change triggers an immediate refresh; legacy caches without provider metadata are treated as mismatched.
  If that refresh fails, the existing stale-cache fallback remains available and identifies the cached provider when known

## Storage (XDG/HOME layout)

| Item    | Path |
|---------|------|
| Config  | `$XDG_CONFIG_HOME/fxrate/config.json`, then `$HOME/.config/fxrate/config.json`, otherwise `./fxrate/config.json` |
| Cache   | `$XDG_DATA_HOME/fxrate/rates.json`, then `$HOME/.local/share/fxrate/rates.json`, otherwise `./fxrate/rates.json` |
| History | `$XDG_DATA_HOME/fxrate/history.db`, then `$HOME/.local/share/fxrate/history.db`, otherwise `./fxrate/history.db` (SQLite: `historical_rates`, `history_coverage`) |

- Config is auto-created with defaults on first run if missing
- Cache is written atomically (uniquely named temp file, fsynced, then renamed) so corruption and concurrent fxrate processes cannot clobber it
- History is written through SQLite transactions (upsert, never delete+reinsert); concurrent CLI runs serialize on rusqlite's default 5s busy timeout rather than failing with `database is locked`
- Override both with the XDG env vars for isolated tests
- The project was renamed from `huobi` to `fxrate` (crate, binary, usage text, and the XDG directory name, which is the single `storage::APP_DIR` constant).
  There is deliberately no migration code: the README tells users to move the old `huobi` config/data directories by hand, and a fresh install simply re-downloads

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
- `provider`: rates source, `frankfurter` (default) or `exchange-api`.
  The alias `exchangeapi` is accepted for both the config key and `-p/--provider` (case-insensitive).
  Invalid values warn and fall back to frankfurter; the CLI `-p/--provider` overrides the config value
- `multi_view`: whether the default multi-currency list is appended after explicit targets.
  Default `true`; an absent field also means enabled (backwards compatible).
  When no targets are given at all, the multi-currency view is always shown regardless of this setting
- `currencies`: default multi-currency view list.
  Available codes depend on the selected provider; unavailable entries warn and are skipped.
  The application does not prevalidate this list against a provider metadata endpoint

## CLI

```text
fxrate [options] AMOUNT SOURCE [TARGET...]
fxrate chart [options] SOURCE TARGET
```

- No targets → **multi-currency view** over the config `currencies` list, always shown regardless of `multi_view`
- With targets: explicit targets are shown first (deduped, order preserved).
  When `multi_view` is enabled, the default list follows, separated by a rule line; when disabled, only the explicit targets are shown.
  Currencies already shown as explicit targets are not repeated in the default list
- The source currency is always excluded
- `-u`, `--update` — force-refresh rates, ignoring cache age
- `-d`, `--date <date>` — convert using the ECB historical rate for `YYYY-MM-DD` instead of the live snapshot.
  The `-p/--provider` selection is ignored (historical rates are always ECB).
  A weekend/holiday date falls back to the previous ECB business day's rate (noted on stderr); a future date (after the latest available ECB day) falls back to that latest day (also noted on stderr); a date with no history at all (before the earliest available) is a runtime error (exit 1); an invalid date format is a usage error (exit 2)
- `-p`, `--provider <name>` — rates source override: `frankfurter` (default) or `exchange-api`.
  Unknown values or unknown options are usage errors (exit 2)
- Amounts must parse as finite numbers; missing arguments and invalid amounts are usage errors (exit 2).
  `-h`/`--help` exits 0
- Exit codes: `0` success · `1` runtime error (fetch failed with no cache, unknown source currency, chart with no history data, convert `--date` with no ECB history on or before that date) · `2` usage error

### Chart command

`fxrate chart SOURCE TARGET` plots `1 SOURCE = x TARGET` over a date range using ECB historical reference rates (charts are always EUR-based cross rates; the convert command and its providers are unaffected).

- `--from <date>` / `--to <date>` — inclusive `YYYY-MM-DD` bounds (default: earliest available data / the last ECB day, extended to the live snapshot date under the live-tail rule).
  Invalid dates or `from > to` are usage errors (exit 2)
- `--format <csv|json|text|auto>` — `auto` (default): text chart when stdout is a terminal (or when `--output` is given), CSV otherwise.
  `csv`/`json` emit text (`date,rate` rows / `{source, target, points}`); `text` emits the text chart
- `--output <path>` — write the chart to a file instead of stdout for every `--format` (`auto`/`text` get the text chart, `csv`/`json` their payload; the file never contains terminal escape sequences)
- `-p`, `--provider <name>` — `ecb` only (default); anything else is a usage error (exit 2)
- Single trading day (ECB or the live tail) → prints `1 SOURCE = x TARGET (date)` instead of a chart; an empty range with neither ECB data nor a live point (e.g. a weekend with no rates cache) is a runtime error (exit 1); a currency with no available history in the requested range is a runtime error (exit 1)
- Terminal charts: a stats panel above a textplots braille chart.
  The panel is a Unicode box titled `SOURCE/TARGET Trend` with Current (with its date),
  High/Low (each with the extreme's date), signed Change (🟢 green up / 🔴 red down, neutral `±0.00%`),
  Average, and Volatility `(high − low) / average`; two stat columns when the box fits the
  terminal width, one stat per line otherwise. The chart has a default canvas of 80
  columns × 15 braille rows (the whole output is 22 lines: panel + chart + two
  axis-label lines); 9 columns on the right are reserved for the y-axis labels that
  trail the braille rows, so they never wrap on a terminal exactly as wide as the
  canvas. The full chart needs the terminal to fit all 22 lines plus 2 spare rows
  (24 rows with the standard panel); on terminals too short for that the chart is
  replaced by a compact chart without axis labels (up to 9 braille rows, only the
  high/low values labeled), or by a one-line sparkline (block characters, bucketed
  and averaged; at most 40 characters) when even that cannot fit. The terminal size
  is read via the `terminal_size` crate (80×24 fallback). The x axis shows `+` tick marks
  with date labels at the start/end dates plus aligned intermediate dates (month starts
  for ranges of two months or more, Mondays from two weeks to two months, every third
  day for shorter ranges, thinned so labels never overlap); the y axis shows dense tick
  labels at round values (1/2/5 × 10^k steps) snapped to cover the data range
- Colors (bright-cyan plotted line, bright-black borders/axes/labels, green/red change) only when stdout is a TTY
  and `NO_COLOR` is unset or empty; `--output` files and piped output never contain escape sequences

## Behavior

- On startup: if the cache is missing, older than `update_interval`, or was produced by a different provider, fetch fresh rates.
  On fetch failure: keep the available cache, print a warning with the failure reason, last rate date, and cached provider when known, and continue.
  Exit 1 only when there is no cache at all and the fetch fails
- Force-update failure always exits 1
- stdout: the conversion table plus a footer line with the rate date — `rates date <date>`.
  When rates were refreshed during the run, the footer is `rates updated: <date>` instead, so the update status and date appear only once, at the bottom.
  On a TTY (stdout, `NO_COLOR` unset or empty) the amounts are bold and the footer is bright black; piped output stays plain
- `--date` convert reuses the ECB history path and all of its sync/fallback/error rules documented under the CLI options above (the rendered rate date is the effective, possibly rolled-back day).
  It applies the same `amount * rate[dst] / rate[src]` EUR-based cross formula as `chart`.
- stderr: notices and warnings (skipped currencies, invalid config, failed refresh fallback) and the history-sync progress spinner.
  `warning:` labels are yellow and `error:` labels are red when stderr is a TTY and `NO_COLOR` is unset or empty — gated independently of stdout, and plain when either stream is redirected
- Chart: on startup, sync the ECB full history when the requested range is not covered by `history_coverage`, when there is no coverage at all, or with `-u/--update`.
  A failed sync warns and falls back to cached data; exit 1 only when no local history exists.
  Covered ranges never touch the network; the coverage check runs only when both `--from` and `--to` are given — a single bound is not separately validated and is clamped to the stored coverage
  The live tail reads `rates.json` as-is — the chart never fetches live rates, so run the convert command first to refresh the snapshot; a missing or unreadable cache just disables the tail (a missing-currency splice prints a warning with the rates date and the chart ends at the last ECB day)
- History syncs (chart first run, uncovered ranges, `-u`, and convert `--date` syncs) render an indicatif spinner on stderr showing the phase (download → parse → SQLite import with a row counter) and finish with the synced date range.
  It is terminal-only: indicatif hides it when stderr is not a user-attended TTY, so piped output, CI, and tests emit nothing

## Conventions

- User-facing messages are in English (only the human conversation around the repo is Chinese)
- Unknown currency in an explicit target → warn and skip (exit 0); the remaining valid conversions still print.
  Unknown source currency → fatal (exit 1), since no conversion is possible.
  Unknown currency in the config list → warn and skip
- If none of the explicit targets are valid, the multi-currency view is shown as a fallback (same rule as "no targets given")
- Invalid config values degrade to defaults with a warning, never a crash; missing or empty `update_interval` is the documented default case
- Currency codes are case-insensitive and normalized to uppercase in memory; the existing config file is not rewritten during normalization

## Releasing

Release flow for a new version:

**Before tagging**: bump the version in `Cargo.toml`, then let cargo refresh `Cargo.lock`'s root entry (edit `Cargo.toml` and run `cargo check --offline`; a hand-edited lock risks mis-firing, and a stale lock breaks `--locked` builds).
Commit it as `release: set crate version to X.Y.Z` and push — the tag points at that commit, so the release flow's step 1 comes after this one.

1. **Tag first, PKGBUILD after.** The tag points at the last code commit on main; the PKGBUILD bump is a follow-up commit.
   This order is required: the PKGBUILD's `source` is GitHub's archive of the tag, so its sha256 can only be computed after the tag exists
2. `git tag vX.Y.Z && git push origin vX.Y.Z` — pushing a `v*` tag triggers `.github/workflows/release.yml`, which builds the four platform binaries.
   A new GitHub release gets auto-generated notes; an existing release has its assets updated

GitHub access: when `gh` is available, prefer it for everything GitHub — `gh` for viewing (runs, releases, repos), and `gh auth setup-git` so `git push` goes over HTTPS through the gh credential helper (SSH on port 22 is often blocked on restricted networks; `gh auth login --web` stores the token in the system keyring).
`gh release view/upload/create` are also used to inspect or repair a release.
Note: `gh` reads `$XDG_CONFIG_HOME` — do not run it with a test home's XDG vars exported in the same shell.

3. Download the tag archive and compute the checksum: `curl -fsSLo /tmp/fxrate.tar.gz https://github.com/YangtseSu/fxrate/archive/refs/tags/vX.Y.Z.tar.gz && sha256sum /tmp/fxrate.tar.gz` — hash the GitHub-served tarball, not a local `git archive` (the PKGBUILD downloads from GitHub)
4. Update `packaging/arch/PKGBUILD`:

- `pkgver=X.Y.Z`, `pkgrel=1` (resets to 1 on every new version)
- `sha256sums=('<new hash>')` — single entry, replaced in full
- Leave everything else (pkgname, arch, source URL pattern, build/check/ package functions) untouched

5. Verify from `packaging/arch/` in a scratch dir: `makepkg -o` (checksum check), then a full `makepkg` build to exercise `build`/`check`/`package` and confirm the produced `.pkg.tar.zst` contains `/usr/bin/fxrate` and the LICENSE
6. Commit `packaging: bump PKGBUILD to vX.Y.Z` and push

The PKGBUILD lives at `packaging/arch/` on main, never in the tagged tree.
Key fields: `arch=('x86_64' 'aarch64')`, `license=('GPL-3.0-only')`, `makedepends=('rust')`, `source=("$url/archive/refs/tags/v$pkgver.tar.gz")`, `cargo build --release --locked` with `cargo test --locked` as the check step.

Repository rename (done): `gh repo rename fxrate` moved `YangtseSu/huobi` to `YangtseSu/fxrate`; `origin` points at the new SSH URL and GitHub redirects the old name.
Because GitHub names the archive's top directory after the repository, the `v0.4.1` tarball is now `fxrate-0.4.1/` and its hash differs from the pre-rename one — `sha256sums` in the PKGBUILD was re-recorded for it.
`v0.5.0` is the first release built as `fxrate`: assets `fxrate-<os>-<arch>[.exe]` plus `checksums.txt`, and it is the repository's "Latest" release.
`v0.3.0`/`v0.4.0`/`v0.4.1` keep their release entries and notes but their assets (`huobi-<os>-<arch>` plus a `checksums.txt` naming those files) were deleted with `gh release delete-asset`.
Re-pushing those tags is not a fix: GitHub runs the `release.yml` stored at the tag, which still builds the old name. The per-tag source archives still resolve, so old versions remain installable from source.

## Testing notes

- Run the full suite with `cargo test --locked` (module unit tests plus `tests/chart.rs` integration tests, which are fully offline: they seed `rates.json` / `history.db` into a fresh XDG home and block the network with `HTTPS_PROXY=http://127.0.0.1:9`). The one exception is `current.rs`, whose unit tests serve canned HTTP responses on a random localhost port — they scrub proxy env vars first so a developer's `HTTP_PROXY` cannot redirect even localhost
- Offline paths: seed a handcrafted `rates.json` (set `fetched_at` to a stale time), include the cached provider when testing provider switching, and block the network, e.g. `HTTPS_PROXY=http://127.0.0.1:9` — refresh fails and the cache fallback is exercised
- Chart live tail: seed `rates.json` dated on/after the coverage end (e.g. a Saturday after a Friday) and assert the appended/replaced row equals the snapshot's `rate[target] / rate[source]`; a snapshot more than 4 days after the last ECB day (stale history) must not splice; a currency missing from the snapshot warns on stderr and the chart ends at the last ECB day; a weekend-only range with a snapshot prints the single-day row
- Historical `--date` convert: a covered date reuses the cache with no network; an uncovered date tries a sync then exits 1 when no local history exists; a weekend/holiday date falls back to the previous business day (noted on stderr, rate date shown is that day); ECB EUR-based cross math matches `chart`
- Fresh XDG dirs simulate a first run: config auto-creation, no-cache failure, and no-history failure (chart exits 1 with no local data)
- When changing related behavior, exercise fresh-cache reuse, stale-cache refresh, provider-change refresh, explicit-first ordering/dedup, offline fallback math, force-update failure, interval handling, invalid config, usage errors, chart coverage/upsert/provider isolation, ECB CSV parsing (N/A, old currency columns, trailing commas), EUR cross rates, single-day charts, and empty ranges
- Chart text UI: stats box lines are rectangular by display width (2-cell emoji counted),
  collapsing to one stat per line on narrow terminals; the chart canvas defaults to
  80×15 braille rows (22 output lines total, plus 2 spare rows of fit margin; 9
  columns reserved for trailing y labels), becomes a label-free compact chart on
  shorter terminals, and a one-line sparkline below that; piped stdout and `--output`
  files contain no escape sequences
- Colors: `render_text(color=true)` pins exact SGR codes (bright-black `90` chrome,
  bright-cyan `96` plotted line, green/red `32`/`31` change), `convert_lines` pins bold
  `1` amounts and the bright-black footer, and `style::label` pins the yellow `33` /
  red `31` warning/error labels; every colored builder also asserts the plain variant
  carries no escape sequences. Stream gates (`style::stdout_color`/`stderr_color`) are
  trivial (TTY + non-empty `NO_COLOR`) and are smoke-tested under a pty rather than
  unit-tested; styling decisions stay explicit at call sites, never auto-detected
  per emission, so redirected output is escape-free by construction

## CI

`.github/workflows/ci.yml` gates every push/PR on `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo build --release --locked`, and `cargo test --locked`.
Before pushing, run `cargo fmt --all` and commit the resulting changes — use `cargo fmt` (not `rustfmt` directly) so the whole crate and the pinned toolchain match CI.
Verify with `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features -- -D warnings`; an unformatted commit fails the CI format check and is fixed in a follow-up `style:` commit
