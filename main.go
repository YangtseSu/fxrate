// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Command huobi is an offline currency conversion CLI.
//
// Rates come from the Frankfurter API (https://frankfurter.dev) and are
// cached locally. On every run the cache age is checked: if it exceeds the
// configured update interval (default 24h) the rates are refreshed
// automatically; a failed refresh falls back to the stale cache with a
// notice. Config and cache follow the XDG directory layout:
//
//	config: $XDG_CONFIG_HOME/huobi/config.json (default ~/.config/huobi/config.json)
//	cache:  $XDG_DATA_HOME/huobi/rates.json (default ~/.local/share/huobi/rates.json)
//
// Usage:
//
//	huobi 100 USD            multi-currency view: USD converted into the configured list
//	huobi 100 USD EUR CNY    explicit targets first, then the default multi-currency list
//	huobi -u 100 USD         force-refresh rates, then convert
package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

const (
	apiBase   = "https://api.frankfurter.dev/v2/rates"
	apiBaseCC = "EUR" // snapshot base currency; cross rates are derived from it
	httpTO    = 15 * time.Second
)

// ---------- Config ----------

// Config is the structure of the config file.
type Config struct {
	// UpdateInterval is the minimum age of cached rates that triggers an
	// automatic refresh, as a Go duration string, e.g. "24h".
	UpdateInterval string `json:"update_interval"`
	// Currencies is the default multi-currency view list, shown when no
	// target currencies are given explicitly.
	Currencies []string `json:"currencies"`
	// MultiView controls whether the default multi-currency list is shown
	// after explicit targets. When no targets are given at all, the
	// multi-currency view is always shown regardless of this setting.
	MultiView *bool `json:"multi_view"`
}

func defaultConfig() *Config {
	return &Config{
		UpdateInterval: "24h",
		MultiView:      new(true),
		Currencies: []string{
			"USD", "EUR", "GBP", "JPY", "CNY", "HKD", "CHF", "AUD", "CAD", "SGD",
			"SEK", "NOK", "DKK", "PLN", "CZK", "HUF", "RON", "KRW", "INR",
			"IDR", "MYR", "PHP", "THB", "ILS", "ISK", "TRY", "MXN", "BRL", "ZAR",
			"NZD",
		},
	}
}

func configPath() string {
	dir, err := os.UserConfigDir()
	if err != nil {
		dir = os.Getenv("HOME")
	}
	return filepath.Join(dir, "huobi", "config.json")
}

func dataPath() string {
	if d := os.Getenv("XDG_DATA_HOME"); d != "" {
		return filepath.Join(d, "huobi", "rates.json")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		home = os.Getenv("HOME")
	}
	return filepath.Join(home, ".local", "share", "huobi", "rates.json")
}

// loadConfig reads the config; writes defaults when missing, and falls back
// to defaults with a warning when the file is malformed.
func loadConfig() *Config {
	b, err := os.ReadFile(configPath())
	if err != nil {
		if !os.IsNotExist(err) {
			fmt.Fprintf(os.Stderr, "warning: failed to read config: %v, using defaults\n", err)
			return defaultConfig()
		}
		cfg := defaultConfig()
		if serr := saveConfig(cfg); serr != nil {
			fmt.Fprintf(os.Stderr, "warning: failed to write default config: %v\n", serr)
		}
		return cfg
	}
	var cfg Config
	if err := json.Unmarshal(b, &cfg); err != nil {
		fmt.Fprintf(os.Stderr, "warning: malformed config: %v, using defaults\n", err)
		return defaultConfig()
	}
	return &cfg
}

func saveConfig(cfg *Config) error {
	if err := os.MkdirAll(filepath.Dir(configPath()), 0o755); err != nil {
		return err
	}
	b, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(configPath(), b, 0o644)
}

// ---------- Rate cache ----------

// RateSnapshot is a cached rates snapshot: base-currency-to-currency rates.
type RateSnapshot struct {
	Base      string             `json:"base"`
	Date      string             `json:"date"`
	FetchedAt time.Time          `json:"fetched_at"`
	Rates     map[string]float64 `json:"rates"`
}

func loadRates() (*RateSnapshot, error) {
	b, err := os.ReadFile(dataPath())
	if err != nil {
		return nil, err
	}
	var s RateSnapshot
	if err := json.Unmarshal(b, &s); err != nil {
		return nil, fmt.Errorf("corrupted rate cache: %w", err)
	}
	return &s, nil
}

func saveRates(s *RateSnapshot) error {
	if err := os.MkdirAll(filepath.Dir(dataPath()), 0o755); err != nil {
		return err
	}
	b, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return err
	}
	tmp := dataPath() + ".tmp"
	if err := os.WriteFile(tmp, b, 0o644); err != nil {
		return err
	}
	return os.Rename(tmp, dataPath())
}

// fetchRates fetches the latest rates snapshot from the Frankfurter API.
func fetchRates() (*RateSnapshot, error) {
	client := &http.Client{Timeout: httpTO}
	resp, err := client.Get(apiBase + "?base=" + apiBaseCC)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		return nil, err
	}
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("API returned %s: %s", resp.Status, strings.TrimSpace(string(body)))
	}
	// The v2 endpoint returns an array of rows: [{"date","base","quote","rate"}, ...]
	var rows []struct {
		Date  string  `json:"date"`
		Base  string  `json:"base"`
		Quote string  `json:"quote"`
		Rate  float64 `json:"rate"`
	}
	if err := json.Unmarshal(body, &rows); err != nil {
		return nil, fmt.Errorf("failed to parse API response: %w", err)
	}
	if len(rows) == 0 {
		return nil, errors.New("API response contained no rates")
	}
	snap := &RateSnapshot{
		Base:      strings.ToUpper(rows[0].Base),
		Date:      rows[0].Date,
		FetchedAt: time.Now(),
		Rates:     make(map[string]float64, len(rows)),
	}
	for _, r := range rows {
		snap.Rates[strings.ToUpper(r.Quote)] = r.Rate
	}
	return snap, nil
}

// ---------- Conversion ----------

// rate returns the rate of 1 unit of cur in the snapshot base currency;
// returns 1 when cur is the base currency itself.
func rate(s *RateSnapshot, cur string) (float64, error) {
	if cur == s.Base {
		return 1, nil
	}
	r, ok := s.Rates[cur]
	if !ok {
		return 0, fmt.Errorf("no rate for currency %s (rates date %s)", cur, s.Date)
	}
	return r, nil
}

func convert(s *RateSnapshot, src, dst string, amount float64) (float64, error) {
	rs, err := rate(s, src)
	if err != nil {
		return 0, err
	}
	rd, err := rate(s, dst)
	if err != nil {
		return 0, err
	}
	return amount * rd / rs, nil
}

// multiView reports whether the default multi-currency list is appended
// after explicit targets. An absent config field (nil) means enabled.
func (c *Config) multiView() bool {
	return c.MultiView == nil || *c.MultiView
}

// dedupeTargets returns the given currencies deduped and in order,
// excluding the source currency.
func dedupeTargets(currencies []string, src string) []string {
	var out []string
	seen := map[string]bool{src: true}
	for _, c := range currencies {
		c = strings.ToUpper(c)
		if !seen[c] {
			seen[c] = true
			out = append(out, c)
		}
	}
	return out
}

// ---------- CLI ----------

func usage() {
	fmt.Fprintf(os.Stderr, `Usage: huobi [options] AMOUNT SOURCE [TARGET...]

Offline currency converter. With no targets, shows the multi-currency view;
explicit targets are listed first, followed by the default multi-currency list.

Options:
  -u, --update  force-refresh rates (ignore cache age)
`)
}

func fatal(format string, a ...any) {
	fmt.Fprintf(os.Stderr, "error: "+format+"\n", a...)
	os.Exit(1)
}

func main() {
	force := flag.Bool("u", false, "force-refresh rates")
	flag.BoolVar(force, "update", false, "force-refresh rates")
	flag.Usage = usage
	flag.Parse()

	args := flag.Args()
	if len(args) < 2 {
		usage()
		os.Exit(2)
	}
	amount, err := strconv.ParseFloat(args[0], 64)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: invalid amount %q\n", args[0])
		os.Exit(2)
	}
	src := strings.ToUpper(args[1])
	explicit := make([]string, 0, len(args)-2)
	for _, a := range args[2:] {
		explicit = append(explicit, strings.ToUpper(a))
	}

	cfg := loadConfig()

	interval := 24 * time.Hour
	if cfg.UpdateInterval != "" {
		if d, err := time.ParseDuration(cfg.UpdateInterval); err == nil && d > 0 {
			interval = d
		} else {
			fmt.Fprintf(os.Stderr, "warning: invalid update_interval=%q in config, falling back to 24h\n", cfg.UpdateInterval)
		}
	}

	snap, lerr := loadRates()
	if lerr != nil {
		if !os.IsNotExist(lerr) {
			fmt.Fprintf(os.Stderr, "warning: failed to read local rates: %v\n", lerr)
		}
		snap = nil
	}

	// Refresh when the cache is missing or stale; fall back to the cache on failure.
	updated := false
	if *force || snap == nil || time.Since(snap.FetchedAt) > interval {
		fresh, ferr := fetchRates()
		switch {
		case ferr == nil:
			if serr := saveRates(fresh); serr != nil {
				fmt.Fprintf(os.Stderr, "warning: failed to save rates cache: %v\n", serr)
			}
			snap = fresh
			updated = true
		case *force:
			fatal("failed to update rates: %v", ferr)
		case snap == nil:
			fatal("no local rate cache and update failed: %v", ferr)
		default:
			fmt.Fprintf(os.Stderr, "warning: failed to update rates: %v; using cached rates (date %s)\n", ferr, snap.Date)
		}
	}

	if _, err := rate(snap, src); err != nil {
		fatal("%v", err)
	}

	// Unknown target currencies are warned about and skipped, so the
	// remaining valid conversions still print.
	type row struct {
		code string
		val  float64
	}
	convertList := func(list []string) []row {
		var rows []row
		for _, t := range list {
			v, err := convert(snap, src, t, amount)
			if err != nil {
				fmt.Fprintf(os.Stderr, "warning: %v, skipped\n", err)
				continue
			}
			rows = append(rows, row{t, v})
		}
		return rows
	}
	explicitRows := convertList(dedupeTargets(explicit, src))
	// The multi-currency view is always shown when no targets are given
	// (or none of them are valid); otherwise it only appears when enabled
	// in the config.
	var multiRows []row
	if len(explicitRows) == 0 || cfg.multiView() {
		multiRows = convertList(dedupeTargets(cfg.Currencies, src))
	}

	// Output: amounts right-aligned; the explicit section starts with
	// "AMOUNT SRC =", the multi-currency section is separated by a blank
	// line and a rule, with its rows indented to the same column.
	amountStr := fmt.Sprintf("%.2f", amount)
	pad := len(amountStr) + len(src) + 3 // width of the "100.00 USD = " prefix
	valW := 0
	for _, r := range explicitRows {
		if w := len(fmt.Sprintf("%.2f", r.val)); w > valW {
			valW = w
		}
	}
	for _, r := range multiRows {
		if w := len(fmt.Sprintf("%.2f", r.val)); w > valW {
			valW = w
		}
	}
	vstr := func(r row) string { return fmt.Sprintf("%*s", valW, fmt.Sprintf("%.2f", r.val)) }
	indent := strings.Repeat(" ", pad)
	for i, r := range explicitRows {
		if i == 0 {
			fmt.Printf("%s %s = %s %s\n", amountStr, src, vstr(r), r.code)
		} else {
			fmt.Printf("%s%s %s\n", indent, vstr(r), r.code)
		}
	}
	if len(multiRows) > 0 {
		if len(explicitRows) > 0 {
			fmt.Println()
			fmt.Println(strings.Repeat("-", pad+valW+4))
		}
		for i, r := range multiRows {
			if i == 0 && len(explicitRows) == 0 {
				fmt.Printf("%s %s = %s %s\n", amountStr, src, vstr(r), r.code)
			} else {
				fmt.Printf("%s%s %s\n", indent, vstr(r), r.code)
			}
		}
	}
	// Footer: when rates were refreshed this run, the update status and the
	// rate date are shown in one line; otherwise just the rate date.
	if updated {
		fmt.Printf("rates updated: %s\n", snap.Date)
	} else {
		fmt.Printf("rates date %s\n", snap.Date)
	}
}
