#!/usr/bin/env python3
"""Offline personal usage budgets from local Claude Code logs. Python 3.9+."""
import json
import math
import os
from pathlib import Path
import re
import sys
import time
from dataclasses import dataclass, field
from enum import Enum
from typing import Optional

PRICING_DATE = "2026-10-06"
MAX_LINE = 4 * 1024 * 1024
MAX_CONFIG = 1024 * 1024
HELP = (
    "quota-lite: offline personal usage budgets (estimates, not vendor quotas)\n\n"
    "python3 quota_lite.py budget set <amount> --per day|week|month "
    "[--unit usd|tokens|requests] [--json]\n"
    "python3 quota_lite.py [--json]\n"
    "python3 quota_lite.py report --by project|model|day [--json]\n"
    "python3 quota_lite.py --tui [--once]\n\n"
    "UTC calendar periods: day, Monday-based week, calendar month.\n"
    "Config: $XDG_CONFIG_HOME/quota-lite/config.json, %APPDATA% on Windows, "
    "or ~/.config/quota-lite/config.json\n"
    "Logs: $CLAUDE_CONFIG_DIR/projects or ~/.claude/projects "
    "(home: HOME, else USERPROFILE)\n"
    "Copilot usage is unknown: local premium-request accounting is not standardized.\n"
    "--tui redraws every 30 seconds; Ctrl-C quits. --once draws one frame.\n"
)
COPILOT_WARNING = (
    "Copilot premium requests, multipliers and costs are unmeasurable from supported local logs. "
    "No message-count proxy is used; requests budgets have unknown usage."
)
COVERAGE_WARNING = (
    "Local log coverage only; subscriptions, discounts, "
    "missing/deleted logs and vendor quotas are not measured."
)
RETENTION_WARNING = (
    "Report window starts before the oldest retained transcript. "
    "Totals and burn rate are lower bounds; remaining and projection may be optimistic. "
    "Claude Code cleanupPeriodDays defaults to 30 days and may be set lower by admins; "
    "missing history may also reflect a new installation."
)
STALE_PRICING_WARNING = (
    "Built-in price table was checked more than 90 days ago; "
    "estimates may use outdated rates. Check current prices and configure overrides."
)
NO_LOGS_WARNING = (
    "No Claude JSONL session logs modified this period; "
    "observed totals do not establish zero actual usage."
)
UNPRICED_WARNING = (
    "messages lack a known price; USD total is a lower bound. "
    "Set prices[model] in config, including cache_write_1h for one-hour cache overrides."
)
PARTIAL_WARNING = (
    "Some log records/paths modified this period were skipped; "
    "remaining, burn and projection are partial-coverage estimates."
)
MODEL_TIP = "consider a lower-priced model for routine tasks."


class Unit(str, Enum):
    USD = "usd"
    TOKENS = "tokens"
    REQUESTS = "requests"


class Period(str, Enum):
    DAY = "day"
    WEEK = "week"
    MONTH = "month"


class By(str, Enum):
    PROJECT = "project"
    MODEL = "model"
    DAY = "day"


def choice(kind, value):
    try:
        return kind(value)
    except (ValueError, TypeError):
        name = "--by" if kind is By else kind.__name__.lower()
        raise ValueError(f"{name} must be {'|'.join(v.value for v in kind)}") from None


def finite_number(value):
    if type(value) not in (int, float):
        raise ValueError("expected a finite number")
    try:
        result = float(value)
    except OverflowError:
        raise ValueError("expected a finite number") from None
    if not math.isfinite(result):
        raise ValueError("expected a finite number")
    return result


@dataclass
class Budget:
    amount: float
    per: Period
    unit: Unit

    def __post_init__(self):
        self.amount = finite_number(self.amount)
        self.per = choice(Period, self.per)
        self.unit = choice(Unit, self.unit)
        if self.amount <= 0:
            raise ValueError("budget amount must be positive")

    def encode(self):
        return {"amount": self.amount, "per": self.per.value, "unit": self.unit.value}


@dataclass
class Price:
    input: float
    output: float
    cache_read: float
    cache_write: float
    cache_write_1h: Optional[float] = None

    def __post_init__(self):
        for name in ("input", "output", "cache_read", "cache_write", "cache_write_1h"):
            value = getattr(self, name)
            if name == "cache_write_1h" and value is None:
                continue
            value = finite_number(value)
            if value < 0:
                raise ValueError("prices must be finite nonnegative USD per million tokens")
            setattr(self, name, value)

    def encode(self):
        return dict(self.__dict__)


def reject_constant(value):
    raise ValueError("nonfinite JSON number")


def decode_json(data):
    return json.loads(data, parse_constant=reject_constant)


@dataclass
class Config:
    budget: Optional[Budget] = None
    prices: dict = field(default_factory=dict)

    def encode(self):
        return {
            "budget": self.budget.encode() if self.budget is not None else None,
            "prices": {name: self.prices[name].encode() for name in sorted(self.prices)},
        }

    @classmethod
    def load(cls, path):
        if not os.path.lexists(path):
            return cls()
        if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_CONFIG:
            raise ValueError("config must be a regular file under 1 MiB")
        try:
            # Bound the read too, in case the file grows after stat().
            with path.open("rb") as source:
                data = source.read(MAX_CONFIG + 1)
            if len(data) > MAX_CONFIG:
                raise ValueError("oversized config")
            raw = decode_json(data)
            if not isinstance(raw, dict):
                raise ValueError("config must be an object")
            budget = raw.get("budget")
            if budget is not None:
                budget = Budget(budget["amount"], budget["per"], budget["unit"])
            prices = raw.get("prices", {})
            if not isinstance(prices, dict):
                raise ValueError("prices must be an object")
            parsed = {}
            for model, rates in prices.items():
                parsed[model] = Price(
                    rates["input"], rates["output"], rates["cache_read"], rates["cache_write"],
                    rates.get("cache_write_1h"),
                )
            return cls(budget, parsed)
        except (ValueError, TypeError, KeyError, AttributeError, RecursionError):
            raise ValueError("invalid config JSON or budget/prices") from None

    def save(self, path):
        # No secrets are stored; refuse symlinks instead of replacing their targets.
        path.parent.mkdir(parents=True, exist_ok=True)
        if os.path.lexists(path) and (path.is_symlink() or not path.is_file()):
            raise ValueError("config must be a regular file")
        path.write_text(json.dumps(self.encode(), indent=2, allow_nan=False), encoding="utf-8")


def home():
    for key in ("HOME", "USERPROFILE"):
        if key in os.environ:
            return Path(os.environ[key])
    raise ValueError("HOME (or USERPROFILE) is not set")


def config_path():
    if "XDG_CONFIG_HOME" in os.environ:
        base = Path(os.environ["XDG_CONFIG_HOME"])
    elif os.name == "nt" and "APPDATA" in os.environ:
        base = Path(os.environ["APPDATA"])
    else:
        base = home() / ".config"
    return base / "quota-lite" / "config.json"


# Public Anthropic list prices, checked 2026-10-06; legacy 3.x snapshot 2025-05-22.
# USD per million tokens. Exact overrides win; unknown families never inherit rates.
PRICE_FAMILIES = {
    "claude-opus-5-5": (4.0, 20.0, 0.05),
    "claude-sonnet-5-5": (2.0, 10.0, 0.1),
    "claude-sonnet-5": (2.0, 10.0, 0.1),
    "claude-fable-5-1": (10.0, 50.0, 0.025),
    "claude-mythos-5-1": (10.0, 50.0, 0.025),
    "claude-fable-5": (10.0, 50.0, 0.1),
    "claude-mythos-5": (10.0, 50.0, 0.1),
    **dict.fromkeys(("claude-opus-5", "claude-opus-4-8", "claude-opus-4-7",
                     "claude-opus-4-6", "claude-opus-4-5"), (5.0, 25.0, 0.1)),
    "claude-haiku-4-5": (1.0, 5.0, 0.1),
    **dict.fromkeys(("claude-sonnet-4-6", "claude-sonnet-4-5", "claude-sonnet-4",
                     "claude-sonnet-4-0", "claude-3-7-sonnet", "claude-3-5-sonnet"),
                    (3.0, 15.0, 0.1)),
    **dict.fromkeys(("claude-opus-4-1", "claude-opus-4", "claude-opus-4-0",
                     "claude-3-opus"), (15.0, 75.0, 0.1)),
    "claude-3-5-haiku": (0.8, 4.0, 0.1),
    "claude-3-haiku": (0.25, 1.25, 0.1),
}


def price(model, config):
    if model in config.prices:
        return config.prices[model]
    family = re.sub(r"-[0-9]{8}$", "", model)
    rates = PRICE_FAMILIES.get(family)
    if rates is None:
        return None
    inp, out, multiplier = rates
    return Price(inp, out, inp * multiplier, inp * 1.25, inp * 2.0)


# Gregorian UTC arithmetic: no timezone dependency, including year zero.
def days(year, month, day):
    year -= int(month <= 2)
    era = year // 400
    yo = year - era * 400
    mp = month + (-3 if month > 2 else 9)
    return era * 146097 + yo * 365 + yo // 4 - yo // 100 + (153 * mp + 2) // 5 + day - 1 - 719468


def civil(day):
    z = day + 719468
    era = z // 146097
    doe = z - era * 146097
    yo = (doe - doe // 1460 + doe // 36524 - doe // 146096) // 365
    year = yo + era * 400
    doy = doe - (365 * yo + yo // 4 - yo // 100)
    mp = (5 * doy + 2) // 153
    d = doy - (153 * mp + 2) // 5 + 1
    m = mp + (3 if mp < 10 else -9)
    return year + int(m <= 2), m, d


def label(timestamp):
    year, month, day = civil(timestamp // 86400)
    return f"{year:04}-{month:02}-{day:02}"


TIMESTAMP = re.compile(
    r"([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2}):([0-9]{2})"
    r"(?:\.[0-9]+)?(Z|[+-][0-9]{2}:[0-9]{2})"
)


def parse_date(value):
    if not isinstance(value, str):
        return None
    match = TIMESTAMP.fullmatch(value)
    if match is None:
        return None
    year, month, day, hour, minute, second = map(int, match.groups()[:6])
    if not 1 <= month <= 12 or not 1 <= day <= 31 or hour > 23 or minute > 59 or second > 59:
        return None
    if civil(days(year, month, day)) != (year, month, day):
        return None
    zone = match.group(7)
    offset = 0
    if zone != "Z":
        zh, zm = int(zone[1:3]), int(zone[4:6])
        if zh > 23 or zm > 59:
            return None
        offset = (zh * 3600 + zm * 60) * (1 if zone[0] == "+" else -1)
    return days(year, month, day) * 86400 + hour * 3600 + minute * 60 + second - offset


def window(now, per):
    per = choice(Period, per)
    day = now // 86400
    if per is Period.DAY:
        return day * 86400, (day + 1) * 86400
    if per is Period.WEEK:
        start = day - (day + 3) % 7
        return start * 86400, (start + 7) * 86400
    year, month, _ = civil(day)
    return (days(year, month, 1) * 86400,
            days(year + int(month == 12), 1 if month == 12 else month + 1, 1) * 86400)


@dataclass
class Event:
    time: int
    project: str
    session: str
    model: str
    tokens: float
    usd: Optional[float]


@dataclass
class Logs:
    events: list = field(default_factory=list)
    oldest_timestamp: Optional[int] = None
    files: int = 0
    skipped: int = 0
    unreadable: int = 0


def billable(record):
    if not isinstance(record, dict) or record.get("type") != "assistant":
        return False
    message = record.get("message")
    if not isinstance(message, dict):
        return False
    model = message.get("model")
    role = message.get("role")
    return isinstance(model, str) and model.startswith("claude-") and (
        not isinstance(role, str) or role == "assistant"
    )


def token_number(usage, key, optional=False):
    if optional and key not in usage:
        return 0.0
    value = usage.get(key)
    if type(value) is not int or not 0 <= value <= 2**64 - 1:
        raise ValueError("invalid token count")
    return float(value)


def event(record, project, session, config):
    if not billable(record):
        return None
    try:
        message = record["message"]
        ident, model, usage = message["id"], message["model"], message["usage"]
        if not isinstance(ident, str) or not isinstance(usage, dict):
            return None
        inp = token_number(usage, "input_tokens")
        out = token_number(usage, "output_tokens")
        read = token_number(usage, "cache_read_input_tokens", True)
        write = token_number(usage, "cache_creation_input_tokens", True)
        timestamp = parse_date(record.get("timestamp"))
        if timestamp is None:
            return None
        creation = usage.get("cache_creation")
        hour = token_number(creation, "ephemeral_1h_input_tokens", True) if isinstance(creation, dict) else 0.0
        if hour > write:
            return None
        rates = price(model, config)
        usd = None
        if rates is not None and (hour == 0 or rates.cache_write_1h is not None):
            hour_rate = rates.cache_write_1h if hour > 0 else 0.0
            usd = (inp * rates.input + out * rates.output + read * rates.cache_read
                   + (write - hour) * rates.cache_write + hour * hour_rate) / 1e6
            if not math.isfinite(usd):
                usd = None
        return ident, Event(timestamp, project, session, model, inp + out + read + write, usd)
    except (KeyError, TypeError, ValueError):
        return None


def regular(path):
    return not path.is_symlink() and path.is_file()


def directory(path):
    return not path.is_symlink() and path.is_dir()


def read_logs(root, config, since):
    logs = Logs()
    messages = {}

    def entries(path):
        try:
            return sorted(path.iterdir(), key=lambda entry: entry.name)
        except OSError:
            logs.unreadable += 1
            return []

    def scan_file(path, project, session):
        try:
            active = path.stat().st_mtime >= max(since, 0)
        except OSError:
            active = True
        try:
            with path.open("rb") as source:
                logs.files += int(active)
                while True:
                    line = source.readline(MAX_LINE + 1)
                    if not line:
                        break
                    if len(line) > MAX_LINE:
                        while not line.endswith(b"\n"):
                            line = source.readline(MAX_LINE + 1)
                            if not line:
                                break
                        logs.skipped += int(active)
                        continue
                    try:
                        record = decode_json(line)
                    except (ValueError, UnicodeError, RecursionError):
                        logs.skipped += int(active)
                        continue
                    if isinstance(record, dict):
                        stamp = parse_date(record.get("timestamp"))
                        if stamp is not None:
                            old = logs.oldest_timestamp
                            logs.oldest_timestamp = stamp if old is None else min(old, stamp)
                    if not active:
                        continue
                    parsed = event(record, project, session, config)
                    if parsed is not None:
                        # Streaming snapshots repeat message.id; last in filename order wins.
                        ident, item = parsed
                        messages[ident] = item
                    elif billable(record) and "usage" in record["message"]:
                        logs.skipped += 1
        except OSError:
            logs.unreadable += int(active)

    if root.exists():
        if not directory(root):
            logs.unreadable += 1
        else:
            for project in entries(root):
                if not directory(project):
                    continue
                for entry in entries(project):
                    if entry.suffix == ".jsonl" and regular(entry):
                        scan_file(entry, project.name, entry.stem)
                    elif directory(entry):
                        sub = entry / "subagents"
                        if directory(sub):
                            for agent in entries(sub):
                                if agent.suffix == ".jsonl" and regular(agent):
                                    scan_file(agent, project.name, f"{entry.name}/{agent.stem}")
    # Match Rust's BTreeMap order for stable floating-point accumulation and output.
    logs.events = [messages[key] for key in sorted(messages)]
    return logs


def event_value(item, unit):
    if unit is Unit.TOKENS:
        return item.tokens
    if unit is Unit.USD:
        return item.usd if item.usd is not None else 0.0
    raise ValueError("requests usage is unmeasurable")


def group(events, unit, by):
    rows = {}
    for item in events:
        if by is By.DAY:
            name = label(item.time)
        elif by is By.MODEL:
            name = item.model
        elif by is By.PROJECT:
            name = item.project
        elif by is None:
            name = f"{item.project}/{item.session}"
        else:
            raise ValueError("invalid grouping")
        row = rows.setdefault(name, {"name": name, "observed": 0.0, "messages": 0, "unpriced_messages": 0})
        row["observed"] += event_value(item, unit)
        row["messages"] += 1
        row["unpriced_messages"] += int(item.usd is None)
    return [rows[key] for key in sorted(rows)]


def projection(spent, budget, elapsed, now):
    if elapsed <= 0 or spent <= 0:
        return None, None
    rate = spent / elapsed * 86400.0
    if rate == 0.0:  # Tiny floating-point values can underflow during division.
        return rate, None
    seconds = max(budget - spent, 0.0) / rate * 86400.0
    hit = None
    if math.isfinite(seconds) and seconds <= 2**63 - 1 - now:
        hit = now + math.ceil(seconds)
    return rate, hit


def build_report(logs, budget, by, now):
    by = choice(By, by)
    start, end = window(now, budget.per)
    events = [item for item in logs.events if start <= item.time < end and item.time <= now]
    unknown = sum(item.usd is None for item in events)
    requests = budget.unit is Unit.REQUESTS
    # Explicit positive float start prevents -0.0 and preserves Rust's accumulation.
    spent = None if requests else 0.0
    if not requests:
        for item in events:
            spent += event_value(item, budget.unit)
    incomplete = requests or (budget.unit is Unit.USD and unknown > 0)
    burn, hit = (None, None) if incomplete else projection(spent, budget.amount, now - start, now)
    sessions = [] if requests else group(events, budget.unit, None)
    sessions.sort(key=lambda row: (-row["observed"], row["name"]))
    models = group(events, Unit.USD, By.MODEL)
    total = sum((row["observed"] for row in models), 0.0)
    tips = []
    if models and total > 0:
        top = max(models, key=lambda row: (row["observed"], row["name"]))
        tips.append(f"{top['name']} accounts for {top['observed'] / total * 100.0:.1f}% of priced spend; {MODEL_TIP}")
    attention = [COPILOT_WARNING, COVERAGE_WARNING]
    if logs.oldest_timestamp is not None and start < logs.oldest_timestamp:
        attention.append(RETENTION_WARNING)
    if now - parse_date(f"{PRICING_DATE}T00:00:00Z") > 90 * 86400:
        attention.append(STALE_PRICING_WARNING)
    if logs.files == 0:
        attention.append(NO_LOGS_WARNING)
    if unknown > 0:
        attention.append(f"{unknown} {UNPRICED_WARNING}")
    if logs.skipped > 0 or logs.unreadable > 0:
        attention.append(PARTIAL_WARNING)
    return {
        "estimate": True, "pricing_date": PRICING_DATE,
        "unit": budget.unit.value, "period": budget.per.value,
        "window_start": label(start), "window_end_exclusive": label(end),
        "budget": budget.amount, "observed_spent": spent,
        "remaining": None if incomplete else budget.amount - spent,
        "burn_per_day": burn, "projected_budget_hit": label(hit) if hit is not None else None,
        "projection_within_window": hit < end if hit is not None else None,
        "files_read": logs.files, "messages": len(events), "unpriced_messages": unknown,
        "skipped_lines": logs.skipped, "unreadable_paths": logs.unreadable,
        "breakdown": [] if requests else group(events, budget.unit, by),
        "unpriced_models": group([item for item in events if item.usd is None], Unit.USD, By.MODEL),
        "expensive_sessions": sessions[:5], "tips": tips, "attention": attention,
    }


def quoted(value):
    return json.dumps(value, ensure_ascii=False, allow_nan=False)


def text_report(report):
    def number(value):
        return "unknown" if value is None else f"{value:.4f}"

    within = report["projection_within_window"]
    lines = [
        f"estimate: true (list prices {report['pricing_date']}, UTC calendar windows)",
        f"budget: {str(report['budget']).removesuffix('.0')} {report['unit']} / {report['period']}",
        f"window: {report['window_start']} .. {report['window_end_exclusive']} (exclusive)",
        f"observed: {number(report['observed_spent'])} | remaining: {number(report['remaining'])} "
        f"| burn/day: {number(report['burn_per_day'])}",
        f"projectedHit: {report['projected_budget_hit'] or 'unknown'} | withinWindow: "
        + ("unknown" if within is None else str(within).lower()),
        f"coverage: {report['files_read']} files, {report['messages']} messages, "
        f"{report['unpriced_messages']} unpriced, {report['skipped_lines']} skipped, "
        f"{report['unreadable_paths']} unreadable",
    ]
    for label_, key in (("breakdown", "breakdown"), ("expensiveSessions", "expensive_sessions"),
                        ("unpricedModels", "unpriced_models")):
        rows = report[key]
        lines.append(f"{label_}[{len(rows)}]{{name,observed,messages,unpriced}}:")
        for row in rows:
            lines.append(f"  {quoted(row['name'])},{row['observed']:.4f},"
                         f"{row['messages']},{row['unpriced_messages']}")
    for tip in report["tips"]:
        lines.append(f"tip: {quoted(tip)}")
    for warning in report["attention"]:
        lines.append(f"attention: {quoted(warning)}")
    return "\n".join(lines) + "\n"


@dataclass
class Args:
    amount: Optional[float] = None
    per: Period = Period.MONTH
    unit: Unit = Unit.USD
    by: By = By.PROJECT
    json: bool = False
    tui: bool = False
    once: bool = False
    help: bool = False


def parse_args(values):
    args = Args()
    pos, setting, reporting = 0, False, False
    if values and values[0] == "budget":
        if len(values) < 2 or values[1] != "set":
            raise ValueError("expected budget set <amount> --per day|week|month")
        if len(values) < 3:
            raise ValueError("missing amount")
        try:
            args.amount = float(values[2])
        except ValueError:
            raise ValueError("invalid amount") from None
        setting, pos = True, 3
    elif values and values[0] == "report":
        reporting, pos = True, 1
    seen = set()
    while pos < len(values):
        key = values[pos]
        pos += 1
        if key in ("--json", "--tui", "--once", "--help", "-h"):
            setattr(args, "help" if key == "-h" else key[2:], True)
        elif key in ("--per", "--unit", "--by"):
            if pos == len(values):
                raise ValueError("missing flag value")
            value = values[pos]
            pos += 1
            if key in seen:
                raise ValueError(f"duplicate {key}")
            seen.add(key)
            kind = {"--per": Period, "--unit": Unit, "--by": By}[key]
            setattr(args, key[2:], choice(kind, value))
        else:
            raise ValueError("unknown command or flag; use --help")
    if setting and "--per" not in seen:
        raise ValueError("budget set requires --per")
    if ((not setting and seen.intersection(("--per", "--unit")))
            or (not reporting and "--by" in seen) or (reporting and "--by" not in seen)
            or (args.tui and (args.json or setting or reporting)) or (args.once and not args.tui)):
        raise ValueError("incompatible flags; use --help")
    if setting:
        Budget(args.amount, args.per, args.unit)
    return args


def prompt_budget():
    def ask(label_, default):
        sys.stderr.write(label_)
        sys.stderr.flush()
        line = sys.stdin.readline()
        if not line:
            raise ValueError("budget prompt cancelled")
        return line.strip() or default

    try:
        amount = float(ask("Personal budget amount: ", ""))
    except ValueError as error:
        if str(error) == "budget prompt cancelled":
            raise
        raise ValueError("invalid amount") from None
    per = choice(Period, ask("Period day|week|month [month]: ", "month"))
    unit = choice(Unit, ask("Unit usd|tokens|requests [usd]: ", "usd"))
    return Budget(amount, per, unit)


def run(values):
    args = parse_args(values)
    if args.help:
        sys.stdout.write(HELP)
        return
    if args.tui and not sys.stdout.isatty() and not args.once:
        raise ValueError("--tui requires a terminal (or --once)")
    path = config_path()
    config = Config.load(path)
    if args.amount is not None:
        config.budget = Budget(args.amount, args.per, args.unit)
        config.save(path)
        print(json.dumps(config.encode(), allow_nan=False) if args.json else "Budget saved.")
        return
    if config.budget is None:
        if not sys.stdin.isatty():
            raise ValueError("no budget configured; run: python3 quota_lite.py budget set "
                             "<amount> --per day|week|month")
        config.budget = prompt_budget()
        config.save(path)
    root = (Path(os.environ["CLAUDE_CONFIG_DIR"]) if "CLAUDE_CONFIG_DIR" in os.environ
            else home() / ".claude") / "projects"
    while True:
        now = int(time.time())
        if now < 0:
            raise ValueError("system clock predates 1970")
        report = build_report(read_logs(root, config, window(now, config.budget.per)[0]),
                              config.budget, args.by, now)
        if args.json:
            print(json.dumps(report, indent=2, ensure_ascii=False, allow_nan=False))
        else:
            if args.tui and sys.stdout.isatty():
                sys.stdout.write("\x1b[2J\x1b[H")
            sys.stdout.write(text_report(report))
        sys.stdout.flush()
        if not args.tui or args.once:
            return
        time.sleep(30)


def main():
    try:
        run(sys.argv[1:])
        return 0
    except KeyboardInterrupt:
        return 130
    except (ValueError, OSError, UnicodeError):
        # Avoid raw paths, transcript contents, or parse-error payloads in errors.
        error = sys.exc_info()[1]
        message = str(error) if isinstance(error, ValueError) else "cannot read or write local files"
        print(f"quota-lite: {message}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
