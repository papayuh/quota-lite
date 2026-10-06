# quota-lite

A small Rust binary for **personal usage budgets**, not vendor quota balances.
It reads local Claude Code session logs, estimates list-price spend, and shows
budget headroom, burn rate, projected exhaustion, project/model/day breakdowns,
and the five most expensive sessions. Works offline. No proxy, routing, login,
credential refresh, telemetry, or runtime network dependencies.

**Copilot is currently unmeasurable.** VS Code and Copilot CLI local logs do not
provide a stable, complete premium-request accounting interface. This version
does not guess billed requests from chat messages or read Copilot credential
files. Requests budgets can be configured but show **unknown**, never zero.
Claude logs alone cannot establish your combined Claude + Copilot spending.

## Install

Rust 1.85+ and Cargo are required. From this repository:

```sh
cargo build --release --locked
# Copy target/release/quota-lite to a directory on your PATH.
cargo test --locked
```

There are exactly two direct dependencies: `serde` (typed config/report encoding)
and `serde_json` (JSON/JSONL parsing). No clap, chrono, async runtime, HTTP client,
terminal library, or network crate. Standard library handles CLI flags, UTC
calendar arithmetic, file IO and terminal refresh. `Cargo.lock` pins all packages.
The locked transitive packages are `serde_core`, `serde_derive`, `proc-macro2`,
`quote`, `syn`, `unicode-ident` (derive macros), and `itoa`, `memchr`, `zmij`
(JSON encoding/parsing internals). Review the lockfile when upgrading.

### Fully offline build

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

Config lives in `$XDG_CONFIG_HOME/quota-lite/config.json` or
`~/.config/quota-lite/config.json`. `budget set` preserves pricing overrides.
Logs come from `$CLAUDE_CONFIG_DIR/projects` or `~/.claude/projects`.
Only `projects/<project>/*.jsonl` and
`projects/<project>/<session>/subagents/*.jsonl` are supported.
Project labels are Claude's directory names, not decoded filesystem paths.
Session ids are file names; parent and subagent sessions are listed separately.
Symlink log entries are skipped. Config symlinks are refused.

All reports use the **current UTC calendar period**: midnight-to-midnight day,
Monday-to-Monday week, or first-to-first calendar month. Reports do not include
past periods or future-dated messages. Total tokens include input, output,
cache-read and cache-creation tokens. Assistant message ids deduplicate streaming
snapshots within each log file (latest snapshot wins); duplicated/copied logs
across different files can double count. Synthetic tests cover this behavior.

Burn is observed usage divided by elapsed time since the period started.
Projected hit date assumes that rate continues; it is UTC, date-only, and can
fall after the period resets (`projection_within_window: false`). Exhausted
budgets show today, not a claimed historical crossing date. Zero usage, zero
elapsed time, unknown prices in USD mode, malformed lines, or unreadable paths
suppress projections. Missing historical logs bias burn downwards. No projection
is a vendor prediction or a promise. `observed_spent` is a **lower bound** when
prices are missing, with remaining/projection unknown. `attention` explicitly
reports incomplete evidence. No logs means zero *observed*, not zero actual.

## Pricing overrides

Built-in public Anthropic list prices checked **2026-10-06**, USD per million
tokens. Source: https://platform.claude.com/docs/en/about-claude/pricing .
Legacy 3.x rates retain the 2025-05-22 snapshot. Prices never auto-update.

| Models (exact ids in `src/config.rs`) | Input | Output | Cache read | Cache write (5 min) |
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

Unknown models are **unpriced**, not silently assigned a default. One-hour cache
writes are unpriced unless an exact override is supplied. Subscription seats,
long-context/fast-mode/residency premiums, batch discounts, taxes, currencies other than USD, and
real billing invoices are not modeled. Override all rates for any such workload;
use a blended cache-write rate if the log mixes cache TTLs. Estimates are not
actual charges, especially on unlimited or subscription plans.

Example config (rate fields are required; nonnegative finite numbers only):

```json
{
  "budget": { "amount": 25, "per": "day", "unit": "usd" },
  "prices": {
    "your-exact-model-id": {
      "input": 1.5, "output": 7.5, "cache_read": 0.15, "cache_write": 1.875
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
streaming deduplication, subagents, config overrides, CLI flags, requests-unknown
behavior, noninteractive setup, leap years, UTC offsets, calendar windows, burn
projection and symlink refusal. End-to-end tests invoke the compiled binary in
isolated directories under `target/`, never real user logs.

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
