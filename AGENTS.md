# Project: huobi — Offline Currency Converter CLI

A Rust offline currency conversion command-line tool.

## Status

Fully implemented and committed; the spec below describes the current behavior.

## Tech stack

- Rust with Cargo and standard library plus `reqwest`, `serde`, `serde_json`, and `chrono`
- Source: `src/main.rs`; manifest: `Cargo.toml`
- Build: `cargo build --release`
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

- `update_interval`: duration string such as `24h` or `1h30m`; auto-refresh threshold (default `24h`).
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
  When `multi_view` is enabled, the default list follows, separated by a rule
  line; when disabled, only the explicit targets are shown. Currencies
  already shown as explicit targets are not repeated in the default list
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
  `rates date <date>`. When rates were refreshed during the run, the footer is
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

## Releasing

Release flow (used for v0.1.0 and v0.1.1):

1. **Tag first, PKGBUILD after.** The tag points at the last code commit on
   main; the PKGBUILD bump is a follow-up commit. This order is required: the
   PKGBUILD's `source` is GitHub's archive of the tag, so its sha256 can only
   be computed after the tag exists
2. `git tag vX.Y.Z && git push origin vX.Y.Z` — pushing a `v*` tag triggers
   `.github/workflows/release.yml`, which builds the four platform binaries
   and creates a GitHub release with auto-generated notes
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

- Offline paths: seed a handcrafted `rates.json` (set `fetched_at` to a stale
  time) and block the network, e.g. `HTTPS_PROXY=http://127.0.0.1:9` — the
  fetch fails and the cache fallback kicks in
- Fresh XDG dirs simulate a first run: config auto-creation, no-cache failure
- Verified end to end: live fetch, explicit-first ordering/dedup, offline
  fallback with correct math, force-update failure, interval honoring,
  invalid-interval fallback, usage errors
