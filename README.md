# quota-lite

A small Rust binary for **personal usage budgets**, not vendor quota balances.
It reads local Claude Code session logs, estimates list-price spend, and shows
budget headroom, burn rate, projected exhaustion, project/model/day breakdowns,
and the five most expensive sessions. Works offline. No proxy, routing, login,
credential refresh, telemetry, or runtime network dependencies.

## Quickstart

Install from source with Rust 1.85+ (`cargo install --path . --locked`), or use a
prebuilt binary from [GitHub Releases](https://github.com/papayuh/quota-lite/releases)
once a version tag has been released. Then:

```sh
quota-lite budget set 25 --per day
quota-lite
quota-lite report --by model --json
```

Example text report (synthetic usage, at noon UTC on 2026-10-06):

```text
estimate: true (list prices 2026-10-06, UTC calendar windows)
budget: 25 usd / day
window: 2026-10-06 .. 2026-10-07 (exclusive)
observed: 10.0000 | remaining: 15.0000 | burn/day: 20.0000
projectedHit: 2026-10-07 | withinWindow: false
coverage: 1 files, 12 messages, 0 unpriced, 0 skipped, 0 unreadable
breakdown[1]{name,observed,messages,unpriced}:
  "synthetic-project",10.0000,12,0
expensiveSessions[1]{name,observed,messages,unpriced}:
  "synthetic-project/session",10.0000,12,0
unpricedModels[0]{name,observed,messages,unpriced}:
tip: "claude-sonnet-4 accounts for 100.0% of priced spend; consider a lower-priced model for routine tasks."
attention: "Copilot premium requests, multipliers and costs are unmeasurable from supported local logs. No message-count proxy is used; requests budgets have unknown usage."
attention: "Local log coverage only; subscriptions, discounts, missing/deleted logs and vendor quotas are not measured."
```

## How it differs from ccusage

[ccusage](https://github.com/ryoppippi/ccusage) focuses on Claude Code usage and
cost reports. quota-lite centers the report on **your budget**: remaining
headroom, observed burn rate, and projected budget exhaustion. It runs fully
offline with just two direct Rust dependencies (`serde` and `serde_json`). It
does not query vendor quota balances or replace a billing statement.

**Copilot is currently unmeasurable.** VS Code and Copilot CLI local logs do not
provide a stable, complete premium-request accounting interface. This version
does not guess billed requests from chat messages or read Copilot credential
files. Requests budgets can be configured but show **unknown**, never zero.
Claude logs alone cannot establish your combined Claude + Copilot spending.

## Install

### Prebuilt binaries

Version tags (`v<version>`, matching `Cargo.toml`) trigger builds for Linux x86-64,
macOS Apple Silicon and Intel, and Windows x86-64. The release attaches `.tar.gz`
archives (Linux/macOS), a `.zip` (Windows), and a `.sha256` sidecar for each.
Download the matching archive and checksum from
[Releases](https://github.com/papayuh/quota-lite/releases). Verify before extracting:

```sh
# Linux, in the download directory:
sha256sum --check quota-lite-v<VERSION>-x86_64-unknown-linux-gnu.tar.gz.sha256
# macOS: shasum -a 256 -c <archive>.sha256
# Windows PowerShell: Get-FileHash <archive>.zip -Algorithm SHA256
# Compare the Windows hash with the downloaded .sha256 file.
```

Extract and copy `quota-lite` (Windows: `quota-lite.exe`) to a directory on your
PATH. Linux binaries require glibc 2.35+; older systems can build from source.
Checksums detect corruption; they are not signatures from an independent source.

### From source

Rust 1.85+ and Cargo are required. From this repository:

```sh
cargo install --path . --locked
cargo test --locked
```

The package metadata is prepared for crates.io, but this change **does not
publish** it. `cargo install quota-lite --locked` becomes available only after
an authorized maintainer publishes with a crates.io token. Source installs and
tag-triggered GitHub binaries do not require that token.

There are exactly two direct dependencies: `serde` (typed config/report encoding)
and `serde_json` (JSON/JSONL parsing). No clap, chrono, async runtime, HTTP client,
terminal library, or network crate. Standard library handles CLI flags, UTC
calendar arithmetic, file IO and terminal refresh. `Cargo.lock` pins all packages.
The locked transitive packages are `serde_core`, `serde_derive`, `proc-macro2`,
`quote`, `syn`, `unicode-ident` (derive macros), and `itoa`, `memchr`, `zmij`
(JSON encoding/parsing internals). Review the lockfile when upgrading.

## Usage

```sh
quota-lite budget set 25 --per day
quota-lite budget set 100 --per week --unit usd
quota-lite budget set 2000000 --per month --unit tokens
quota-lite budget set 300 --per month --unit requests
quota-lite
quota-lite report --by project
quota-lite report --by model --json
quota-lite report --by day
quota-lite --tui          # redraw every 30 seconds; Ctrl-C quits
quota-lite --tui --once   # one frame, also works without a terminal
```

First run prompts for amount, period (default month), and unit (default USD) when
stdin is a terminal. Without a terminal, missing config produces an actionable
error; no hanging prompt. JSON goes only to stdout; prompts/errors go to stderr.
Exit status: 0 success, 2 invalid arguments/config or IO failure.

Config lives in `$XDG_CONFIG_HOME/quota-lite/config.json`, then
`%APPDATA%\quota-lite\config.json` on Windows, then
`~/.config/quota-lite/config.json`. `budget set` preserves pricing overrides.
Logs come from `$CLAUDE_CONFIG_DIR/projects` or `~/.claude/projects`.
Home is `HOME`, else `USERPROFILE` (Windows); it is only needed when the
overrides above are unset.
Only Claude API assistant entries (model ids starting `claude-`) count as usage;
synthetic, missing-model, non-Claude, user and tool records are excluded.
Only `projects/<project>/*.jsonl` and
`projects/<project>/<session>/subagents/*.jsonl` are supported.
Project labels are Claude's directory names, not decoded filesystem paths.
Session ids are file names; parent and subagent sessions are listed separately.
Symlink log entries are skipped. Config symlinks are refused.

All reports use the **current UTC calendar period**: midnight-to-midnight day,
Monday-to-Monday week, or first-to-first calendar month. Reports do not include
past periods or future-dated messages. Total tokens include input, output,
cache-read and cache-creation tokens. Assistant message ids deduplicate across
all log files, covering streaming snapshots and history copied by resumed
sessions (latest snapshot wins; files are read in name order). Log files not
modified since the period started are excluded from usage accounting, since logs
are append-only. Retained files are still scanned for timestamps to establish
the oldest available transcript, including non-usage records. This can make
reports slower for large retained histories. Synthetic tests cover this behavior.

Burn is observed usage divided by elapsed time since the period started.
Projected hit date assumes that rate continues; it is UTC, date-only, and can
fall after the period resets (`projection_within_window: false`). Exhausted
budgets show today, not a claimed historical crossing date. Zero usage, zero
elapsed time, or unknown prices in USD mode suppress projections. Malformed
lines or unreadable paths in files modified this period are counted and flagged
in `attention`; remaining, burn and projection are still shown as
partial-coverage estimates. Claude Code deletes old transcripts according to
`cleanupPeriodDays` (30 days by default; admins may set it lower). If the window
starts before the oldest retained transcript timestamp, `attention` says totals
and burn rate are **lower bounds**; remaining and projection may be optimistic.
This is a coverage warning, not proof of deletion: a new installation can also
have short history, and gaps after the oldest transcript cannot be detected.
No logs still gets the separate no-usage-evidence warning. No projection
is a vendor prediction or a promise. `observed_spent` is a **lower bound** when
prices are missing, with remaining/projection unknown. `attention` explicitly
reports incomplete evidence. No logs means zero *observed*, not zero actual.

## Pricing overrides

Built-in public Anthropic list prices checked **2026-10-06**, USD per million
tokens. Source: https://platform.claude.com/docs/en/about-claude/pricing .
Legacy 3.x rates retain the 2025-05-22 snapshot. Prices never auto-update.
Once the built-in table's checked date is more than 90 days old, `attention`
warns that rates may be stale. Review current prices and add exact-model overrides.

| Model families (`src/config.rs`; [official source](https://platform.claude.com/docs/en/about-claude/pricing), checked 2026-10-06; legacy 3.x snapshot 2025-05-22) | Input | Output | Cache read | Cache write (5 min) |
|---|---:|---:|---:|---:|
| Opus 5.5 | 4 | 20 | 0.20 | 5 |
| Sonnet 5.5, 5 | 2 | 10 | 0.20 | 2.50 |
| Fable 5.1, Mythos 5.1 | 10 | 50 | 0.25 | 12.50 |
| Fable 5, Mythos 5 | 10 | 50 | 1 | 12.50 |
| Opus 5, 4.8, 4.7, 4.6, 4.5 | 5 | 25 | 0.50 | 6.25 |
| Haiku 4.5 | 1 | 5 | 0.10 | 1.25 |
| Sonnet 4.6, 4.5, 4, 3.7, 3.5 | 3 | 15 | 0.30 | 3.75 |
| Opus 4.1, 4, Opus 3 | 15 | 75 | 1.50 | 18.75 |
| Haiku 3.5 | 0.80 | 4 | 0.08 | 1 |
| Haiku 3 | 0.25 | 1.25 | 0.025 | 0.3125 |

Known model families match aliases and `-YYYYMMDD` snapshots; arbitrary suffixes
and unknown future families never inherit a price. Exact config overrides win.
One-hour cache writes cost **2 × input price** per million tokens (official table,
checked 2026-10-06), while five-minute writes cost 1.25 × input price. Nested TTL
counts partition the total cache-creation tokens; they are not added again.
Unknown models are **unpriced**, not silently assigned a default. Both summary
and JSON list `unpricedModels`/`unpriced_models` with counts and an override hint.
Subscription seats,
long-context/fast-mode/residency premiums, batch discounts, taxes, currencies other than USD, and
real billing invoices are not modeled. Override all rates for any such workload;
set `cache_write_1h` as well as the five-minute `cache_write` for mixed cache TTLs.
Older overrides without `cache_write_1h` still work for five-minute writes but
are unpriced for one-hour writes. Estimates are not
actual charges, especially on unlimited or subscription plans.

Example config (rate fields are required; nonnegative finite numbers only):

```json
{
  "budget": { "amount": 25, "per": "day", "unit": "usd" },
  "prices": {
    "your-exact-model-id": {
      "input": 1.5, "output": 7.5, "cache_read": 0.15, "cache_write": 1.875,
      "cache_write_1h": 3
    }
  }
}
```

## Privacy and scope

No runtime network calls. No telemetry. No credential-file or keychain access.
Only the tool's config and selected session JSONL logs are read. Transcripts may
contain sensitive data: JSON records are parsed locally but prompt text, tool
arguments, credentials, raw JSON, and parse-error contents are never printed or
persisted. Output does include project directory labels, model ids and session
file names; treat those as potentially sensitive. `--json` is an aggregate
report, not a transcript export. Malformed or >4 MiB lines are skipped with
counts; config is limited to 1 MiB. Config and reports remain local unless you
choose to share them. Run as your normal user, not with elevated privileges.
Concurrent budget writes are not coordinated; configure from one process.

## Tests

```sh
cargo fmt --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
```

Synthetic fixtures only: parser/schema validation, pricing, unknown models,
streaming deduplication, non-API exclusions, mixed cache TTLs, dated model-family
matching, subagents, config overrides, CLI flags, requests-unknown
behavior, noninteractive setup, leap years, UTC offsets, calendar windows, burn
projection and symlink refusal. End-to-end tests invoke the compiled binary in
isolated directories under `target/`, never real user logs.

## Offline build preparation

### Vendoring

On a connected preparation machine using this exact lockfile:

```sh
cargo vendor --locked vendor > vendor-config.toml
```

Transfer the source, `Cargo.lock`, and `vendor/` to the offline machine. Create
`.cargo/config.toml` in the repository (these machine-specific files are ignored):

```toml
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"

[net]
offline = true
```

Then `cargo build --release --locked --offline` and
`cargo test --locked --offline`. No Cargo download is needed. Vendoring is a
build preparation step; references such as shunt are never vendored.

### Internal registry mirror

Instead of vendoring, create `.cargo/config.toml` using your approved registry URL:

```toml
[source.crates-io]
replace-with = "internal"

[source.internal]
registry = "sparse+https://artifactory.example.invalid/api/cargo/approved/index/"
```

The trailing `/` matters. Use your organization's normal Cargo credential
provider; do not put tokens in this repository. Populate Cargo's cache from the
mirror, then build/test with `--locked --offline`. Cargo may use the network
while preparing dependencies; **the built quota-lite binary never does**.
A mirror must contain the pinned versions (including transitives). Dependencies
are not dynamically replaced: changes to allowed versions require updating and
reviewing `Cargo.lock` before transfer.

## Inspiration and license

- [pleaseai/shunt](https://github.com/pleaseai/shunt) (MIT/Apache-2.0): studied
  `src/oauth_usage.rs` and dashboard metrics design. Borrowed the separation of
  measured aggregate evidence from presentation and explicit window boundaries;
  no endpoint, routing, credential logic, or source code copied.
- [quota-axi](https://www.npmjs.com/package/quota-axi): studied installed CLI/help
  and README. Inspired compact TOON-like rows, JSON parity, unknown/attention
  evidence, burn/runway presentation, and a dependency-free refresh view.
  Default rows use JSON-quoted names; this is not a full TOON implementation.

This is a standalone implementation, not a literal source fork. MIT; see LICENSE.
