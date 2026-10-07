# quota-lite

**Personal usage budgets**, not vendor quota balances. One Python file. It reads
local Claude Code session logs, estimates list-price spend, and shows remaining
budget, burn rate, projected exhaustion, project/model/day breakdowns, and the
five most expensive sessions. Fully offline. Python 3.9+, standard library only.
No pip install, account login, telemetry, or runtime network calls.

## Quickstart

Download [quota_lite.py](quota_lite.py) (use GitHub's **Raw → Save as**), then run:

```sh
python3 quota_lite.py budget set 25 --per day
python3 quota_lite.py
python3 quota_lite.py report --by model --json
```

On Windows, use `py -3` or `python` instead of `python3`. No other files or
packages are required to run it. Keep your downloaded copy wherever you like.

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
offline with **zero third-party dependencies**. It does not query vendor quota
balances or replace a billing statement.

**Copilot is currently unmeasurable.** VS Code and Copilot CLI local logs do not
provide a stable, complete premium-request accounting interface. This version
does not guess billed requests from chat messages or read Copilot credential
files. Requests budgets can be configured but show **unknown**, never zero.
Claude logs alone cannot establish combined Claude + Copilot spending.

## Usage

```sh
python3 quota_lite.py budget set 25 --per day
python3 quota_lite.py budget set 100 --per week --unit usd
python3 quota_lite.py budget set 2000000 --per month --unit tokens
python3 quota_lite.py budget set 300 --per month --unit requests
python3 quota_lite.py
python3 quota_lite.py report --by project
python3 quota_lite.py report --by model --json
python3 quota_lite.py report --by day
python3 quota_lite.py --tui          # redraw every 30 seconds; Ctrl-C quits
python3 quota_lite.py --tui --once   # one frame, also works without a terminal
```

First run prompts for amount, period (default month), and unit (default USD) when
stdin is a terminal. Without a terminal, missing config produces an actionable
error; no hanging prompt. JSON goes only to stdout; prompts/errors go to stderr.
Exit status: 0 success, 2 invalid arguments/config or IO failure, 130 interrupted.
Unit, period, and grouping choices are enums; invalid values are errors, not
silent fallbacks. Budgets must be finite positive numbers.

Config lives in `$XDG_CONFIG_HOME/quota-lite/config.json`, then
`%APPDATA%\quota-lite\config.json` on Windows, then
`~/.config/quota-lite/config.json`. `budget set` preserves pricing overrides.
Logs come from `$CLAUDE_CONFIG_DIR/projects` or `~/.claude/projects`.
Home is `HOME`, else `USERPROFILE`; it is only needed when the overrides above
are unset. Existing config from the former Rust version is compatible.

Only Claude API assistant entries (model ids starting `claude-`) count as usage;
synthetic, missing-model, non-Claude, user and tool records are excluded.
Only `projects/<project>/*.jsonl` and
`projects/<project>/<session>/subagents/*.jsonl` are supported.
Project labels are Claude's directory names, not decoded filesystem paths.
Session ids are file names; parent and subagent sessions are listed separately.
Symlink log entries are skipped. Config symlinks are refused.

## Coverage and projections

All reports use the **current UTC calendar period**: midnight-to-midnight day,
Monday-to-Monday week, or first-to-first calendar month. Reports do not include
past periods or future-dated messages. Total tokens include input, output,
cache-read and cache-creation tokens. Assistant message ids deduplicate across
all log files, covering streaming snapshots and history copied by resumed
sessions (latest snapshot wins; files are read in name order).

Log files not modified since the period started are excluded from usage
accounting, since logs are append-only. Their modification times still mark the
oldest retained transcript.

Burn is observed usage divided by elapsed time since the period started.
Projected hit date assumes that rate continues; it is UTC, date-only, and can
fall after the period resets (`projection_within_window: false`). Exhausted
budgets show today, not a claimed historical crossing date. Zero usage, zero
elapsed time, or unknown prices in USD mode suppress projections.

Malformed lines or unreadable paths in files modified this period are counted
and flagged in `attention`; remaining, burn and projection are still shown as
partial-coverage estimates. Claude Code deletes old transcripts according to
`cleanupPeriodDays` (30 days by default; admins may set it lower). If the window
starts before the oldest retained transcript's modification time (cleanup
deletes by modification time, so copied history in resumed sessions does not
count), `attention` says totals and burn rate are **lower bounds**; remaining
and projection may be optimistic.
This is a coverage warning, not proof of deletion: a new installation can also
have short history, and gaps after the oldest transcript cannot be detected.

No projection is a vendor prediction or promise. `observed_spent` is a **lower
bound** when prices are missing, with remaining/projection unknown. No logs means
zero *observed*, not zero actual; it gets a separate attention warning.

## Pricing overrides

Built-in public Anthropic list prices checked **2026-10-06**, USD per million
tokens. Source: https://platform.claude.com/docs/en/about-claude/pricing .
Legacy 3.x rates retain the 2025-05-22 snapshot. Prices never auto-update.
Once the built-in table's checked date is more than 90 days old, `attention`
warns that rates may be stale. Review current prices and add exact-model overrides.

| Model families (`quota_lite.py`; checked 2026-10-06; legacy 3.x snapshot 2025-05-22) | Input | Output | Cache read | Cache write (5 min) |
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
Subscription seats, long-context/fast-mode/residency premiums, batch discounts,
taxes, currencies other than USD, and real billing invoices are not modeled.
Override all rates for any such workload; set `cache_write_1h` as well as the
five-minute `cache_write` for mixed cache TTLs. Older overrides without
`cache_write_1h` still work for five-minute writes but are unpriced for one-hour
writes. Estimates are not actual charges, especially on subscription plans.

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
persisted. Output includes project directory labels, model ids and session file
names; treat those as potentially sensitive. `--json` is an aggregate report,
not a transcript export.

Malformed or >4 MiB lines are skipped with counts; config is limited to 1 MiB.
Config and reports remain local unless you choose to share them. Run as your
normal user, not with elevated privileges. Concurrent budget writes are not
coordinated; configure from one process.

## Tests

```sh
python3 -m py_compile quota_lite.py test_quota_lite.py
python3 -m unittest -v
```

Synthetic fixtures only. Tests cover parser/schema validation, pricing, unknown
models, streaming deduplication, non-API exclusions, cache TTLs, dated aliases,
subagents, config overrides, invalid flags, requests-unknown behavior, prompts,
noninteractive setup, path precedence, leap years, UTC offsets, calendar windows,
burn/projection, retention warnings, stale-price boundaries, positive zero,
malformed/oversized records and symlink refusal. CLI tests invoke the script in
isolated directories under `target/`, never real user logs. CI runs on Linux,
macOS and Windows with Python 3.9 and the latest stable Python 3.x.

The Python port was also checked against the former Rust version on local logs
before removal: totals, coverage, breakdowns, sessions and attention matched;
with the same clock, the entire JSON report matched. No real logs are committed.

## Inspiration and license

- [pleaseai/shunt](https://github.com/pleaseai/shunt) (MIT/Apache-2.0): studied
  `src/oauth_usage.rs` and dashboard metrics design. Borrowed the separation of
  measured aggregate evidence from presentation and explicit window boundaries;
  no endpoint, routing, credential logic, or source code copied.
- [quota-axi](https://www.npmjs.com/package/quota-axi): inspired compact TOON-like
  rows, JSON parity, unknown/attention evidence, burn/runway presentation, and a
  dependency-free refresh view. Default rows use JSON-quoted names; this is not
  a full TOON implementation.

Standalone implementation, not a literal source fork. MIT; see LICENSE.
