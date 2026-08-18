# Project: huobi — Offline Currency Converter CLI

A Rust offline currency conversion command-line tool.

## Scope

This file documents the current behavior and project conventions.
`src/main.rs` is the implementation source of truth; verify behavior against
the source and tests when changing this document.

## Tech stack

- Rust with Cargo and standard library plus `reqwest`, `serde`, `serde_json`, and `chrono`
- Source: `src/main.rs`; manifest: `Cargo.toml`
- Build: `cargo build --release --locked`
- Test: `cargo test --locked`
- Cargo build artifact: `target/release/huobi`; Cargo's `target/` directory is gitignored

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

- `date` = the rates' business date from the API; `fetched_at` = local fetch
  time; both are stored in the cache
- The cache records the provider that produced it. A provider change triggers
  an immediate refresh; legacy caches without provider metadata are treated as
  mismatched. If that refresh fails, the existing stale-cache fallback remains
  available and identifies the cached provider when known

## Storage (XDG/HOME layout)

| Item   | Path |
|--------|------|
| Config | `$XDG_CONFIG_HOME/huobi/config.json`, then `$HOME/.config/huobi/config.json`, otherwise `./huobi/config.json` |
| Cache  | `$XDG_DATA_HOME/huobi/rates.json`, then `$HOME/.local/share/huobi/rates.json`, otherwise `./huobi/rates.json` |

- Config is auto-created with defaults on first run if missing
- Cache is written atomically (temp file + rename) to avoid corruption
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
```

- No targets → **multi-currency view** over the config `currencies` list,
  always shown regardless of `multi_view`
- With targets: explicit targets are shown first (deduped, order preserved).
  When `multi_view` is enabled, the default list follows, separated by a rule
  line; when disabled, only the explicit targets are shown. Currencies
  already shown as explicit targets are not repeated in the default list
- The source currency is always excluded
- `-u`, `--update` — force-refresh rates, ignoring cache age
- `-p`, `--provider <name>` — rates source override: `frankfurter` (default) or
  `exchange-api`. Unknown values or unknown options are usage errors (exit 2)
- Amounts must parse as finite numbers; missing arguments and invalid amounts
  are usage errors (exit 2). `-h`/`--help` exits 0
- Exit codes: `0` success · `1` runtime error (fetch failed with no cache,
  unknown source currency) · `2` usage error

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

- Run the unit suite with `cargo test --locked`
- Offline paths: seed a handcrafted `rates.json` (set `fetched_at` to a stale
  time), include the cached provider when testing provider switching, and block
  the network, e.g. `HTTPS_PROXY=http://127.0.0.1:9` — refresh fails and the
  cache fallback is exercised
- Fresh XDG dirs simulate a first run: config auto-creation and no-cache
  failure
- When changing related behavior, exercise fresh-cache reuse, stale-cache
  refresh, provider-change refresh, explicit-first ordering/dedup, offline
  fallback math, force-update failure, interval handling, invalid config, and
  usage errors
