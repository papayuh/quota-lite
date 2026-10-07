"""Synthetic-only regression tests. Run: python3 -m unittest -v."""
import contextlib
import io
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import quota_lite as q

ROOT = Path(__file__).resolve().parent
FIXTURES = ROOT / "tests" / "fixtures"
NOW = q.parse_date("2026-10-06T12:00:00Z")


class Sandbox(unittest.TestCase):
    def setUp(self):
        (ROOT / "target").mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "target")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / "claude/projects/synthetic-project"
        self.project.mkdir(parents=True)
        self.env = os.environ.copy()
        self.env.update(HOME=str(self.root), XDG_CONFIG_HOME=str(self.root / "config"),
                        CLAUDE_CONFIG_DIR=str(self.root / "claude"))
        self.path = self.root / "config/quota-lite/config.json"

    def cli(self, *args, env=None):
        return subprocess.run([sys.executable, str(ROOT / "quota_lite.py"), *args],
                              env=self.env if env is None else env, input="",
                              capture_output=True, text=True, timeout=30)

    def json(self, *args):
        result = self.cli(*args)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("SYNTHETIC_SECRET", result.stdout)
        return json.loads(result.stdout)

    def fixture(self, name):
        stamp = q.label(int(q.time.time())) + "T00:00:00Z"
        data = (FIXTURES / name).read_text(encoding="utf-8")
        self.project.joinpath("session.jsonl").write_text(
            data.replace("2024-02-29T12:00:00Z", stamp), encoding="utf-8")

    def set_budget(self, unit="usd", per="day", amount="25"):
        self.assertEqual(self.cli("budget", "set", amount, "--per", per, "--unit", unit).returncode, 0)

    def update_config(self, transform):
        config = json.loads(self.path.read_text())
        transform(config)
        self.path.write_text(json.dumps(config), encoding="utf-8")

    def test_dedup_unknown_prices_and_units(self):
        self.fixture("session.jsonl")
        self.assertEqual(self.cli("--json").returncode, 2)
        self.set_budget()
        report = self.json("report", "--by", "model", "--json")
        self.assertEqual(report["messages"], 2)
        self.assertEqual(report["unpriced_messages"], 1)
        self.assertEqual(report["unpriced_models"][0]["name"], "claude-future-model")
        self.assertAlmostEqual(report["observed_spent"], 0.00756)
        self.assertIsNone(report["remaining"])
        self.assertIsNone(report["projected_budget_hit"])
        self.assertEqual(len(report["breakdown"]), 2)
        self.assertTrue(any("Set prices[model]" in warning for warning in report["attention"]))
        self.assertIn("claude-future-model", self.cli().stdout)
        self.set_budget("tokens", "week", "10000")
        self.assertEqual(self.json("--json")["observed_spent"], 2400.0)
        self.set_budget("requests", "month", "100")
        report = self.json("--json")
        self.assertIsNone(report["observed_spent"])
        self.assertIsNone(report["burn_per_day"])
        self.assertEqual(report["breakdown"], [])
        self.assertIn(q.COPILOT_WARNING, report["attention"])
        self.assertEqual(self.cli("--tui", "--once").returncode, 0)

    def test_no_usage_is_positive_zero(self):
        for unit in ("usd", "tokens"):
            self.set_budget(unit, "month")
            report = self.json("--json")
            self.assertEqual(math.copysign(1.0, report["observed_spent"]), 1.0)
            self.assertEqual(report["observed_spent"], 0.0)
            self.assertEqual(report["remaining"], 25.0)
            self.assertIsNone(report["burn_per_day"])
            self.assertIsNone(report["projected_budget_hit"])
            self.assertNotIn("-0.0000", self.cli().stdout)
            self.assertIn(q.NO_LOGS_WARNING, report["attention"])

    def test_overrides_malformed_subagents_and_inactive_files(self):
        self.set_budget(per="month")
        rates = {"input": 1, "output": 1, "cache_read": 1, "cache_write": 1}
        self.update_config(lambda c: c["prices"].update({"claude-sonnet-4-20250514": rates}))
        self.fixture("message.json")
        self.assertEqual(self.json("--json")["observed_spent"], 0.0018)
        sub = self.project / "session/subagents"
        sub.mkdir(parents=True)
        (sub / "agent.jsonl").write_bytes((self.project / "session.jsonl").read_bytes())
        report = self.json("--json")
        self.assertEqual(report["messages"], 1)
        self.assertEqual(report["files_read"], 2)
        (self.project / "session.jsonl").write_text("not json\n", encoding="utf-8")
        report = self.json("--json")
        self.assertEqual(report["skipped_lines"], 1)
        self.assertEqual(report["messages"], 1)
        self.assertIn(q.PARTIAL_WARNING, report["attention"])
        self.assertIsNotNone(report["remaining"])
        os.utime(self.project / "session.jsonl", (86400, 86400))
        report = self.json("--json")
        self.assertEqual(report["skipped_lines"], 0)
        self.assertEqual(report["files_read"], 1)
        self.set_budget(amount="50")
        self.assertEqual(self.json("--json")["observed_spent"], 0.0018)  # Overrides survive budget set.
        self.update_config(lambda c: c["prices"]["claude-sonnet-4-20250514"].update(input=-1))
        self.assertEqual(self.cli("--json").returncode, 2)

    def test_mixed_cache_non_api_entries_and_old_overrides(self):
        self.set_budget()
        self.fixture("mixed-cache.jsonl")
        report = self.json("--json")
        self.assertEqual(report["messages"], 1)
        self.assertEqual(report["unpriced_messages"], 0)
        self.assertEqual(report["skipped_lines"], 0)
        self.assertAlmostEqual(report["observed_spent"], 0.00801)
        rates = {"input": 3, "output": 15, "cache_read": 0.3, "cache_write": 3.75}
        self.update_config(lambda c: c["prices"].update({"claude-sonnet-4-20250514": rates}))
        self.assertEqual(self.json("--json")["unpriced_messages"], 1)
        self.update_config(lambda c: c["prices"]["claude-sonnet-4-20250514"].update(cache_write_1h=6))
        self.assertEqual(self.json("--json")["unpriced_messages"], 0)

    def test_config_rejects_invalid_schema_nonfinite_and_oversized(self):
        self.path.parent.mkdir(parents=True)
        for budget in ({"amount": 25, "per": "year", "unit": "usd"},
                       {"amount": 25, "per": "day", "unit": "dollars"},
                       {"amount": True, "per": "day", "unit": "usd"},
                       {"amount": 0, "per": "day", "unit": "usd"}):
            self.path.write_text(json.dumps({"budget": budget}), encoding="utf-8")
            self.assertEqual(self.cli("--json").returncode, 2)
        for raw in ('{"budget":NaN}', '[]', '{"prices":[]}', '{"prices":{"secret":null}}',
                    "SYNTHETIC_SECRET", " " * (q.MAX_CONFIG + 1)):
            self.path.write_text(raw, encoding="utf-8")
            result = self.cli("--json")
            self.assertEqual(result.returncode, 2)
            self.assertNotIn("SYNTHETIC_SECRET", result.stderr)

    @unittest.skipIf(os.name == "nt", "Windows symlinks require privileges")
    def test_refuses_log_config_and_directory_symlinks(self):
        self.fixture("message.json")
        (self.project / "copy.jsonl").symlink_to("session.jsonl")
        (self.project.parent / "copy-project").symlink_to(self.project, target_is_directory=True)
        self.set_budget()
        self.assertEqual(self.json("--json")["files_read"], 1)
        backup = self.path.with_suffix(".backup")
        self.path.rename(backup)
        self.path.symlink_to(backup)
        self.assertEqual(self.cli("--json").returncode, 2)
        self.assertEqual(self.cli("budget", "set", "2", "--per", "day").returncode, 2)

    def test_lazy_home_and_userprofile_fallback(self):
        env = self.env.copy()
        for key in ("HOME", "USERPROFILE", "APPDATA", "XDG_CONFIG_HOME", "CLAUDE_CONFIG_DIR"):
            env.pop(key, None)
        overrides = dict(env, XDG_CONFIG_HOME=str(self.root / "config"),
                         CLAUDE_CONFIG_DIR=str(self.root / "claude"))
        self.assertEqual(self.cli("budget", "set", "5", "--per", "day", env=overrides).returncode, 0)
        self.assertEqual(self.cli("--json", env=overrides).returncode, 0)
        self.assertEqual(self.cli("--json", env=env).returncode, 2)
        profile = dict(env, USERPROFILE=str(self.root))
        self.assertEqual(self.cli("budget", "set", "5", "--per", "day", env=profile).returncode, 0)
        self.assertTrue((self.root / ".config/quota-lite/config.json").is_file())
        logs = self.root / ".claude/projects/p"
        logs.mkdir(parents=True)
        (logs / "s.jsonl").write_text("", encoding="utf-8")
        result = self.cli("--json", env=profile)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["files_read"], 1)
        if os.name == "nt":
            appdata = dict(profile, APPDATA=str(self.root / "appdata"))
            self.assertEqual(self.cli("budget", "set", "5", "--per", "day", env=appdata).returncode, 0)
            self.assertTrue((self.root / "appdata/quota-lite/config.json").is_file())

    def test_oldest_timestamp_includes_inactive_nonbillable_records(self):
        old = self.project / "old.jsonl"
        old.write_text('{"timestamp":"2024-01-01T00:00:00Z","type":"user"}\n', encoding="utf-8")
        os.utime(old, (86400, 86400))
        logs = q.read_logs(self.project.parent, q.Config(), q.parse_date("2024-02-01T00:00:00Z"))
        self.assertEqual(logs.oldest_timestamp, q.parse_date("2024-01-01T00:00:00Z"))
        self.assertEqual(logs.files, 0)
        self.assertEqual(logs.events, [])

    def test_oversized_invalid_utf8_and_malformed_records(self):
        self.set_budget()
        self.fixture("message.json")
        valid = (self.project / "session.jsonl").read_bytes()
        (self.project / "session.jsonl").write_bytes(b"x" * (q.MAX_LINE + 1) + b"\n\xff\n" + valid)
        report = self.json("--json")
        self.assertEqual(report["skipped_lines"], 2)
        self.assertEqual(report["messages"], 1)

    def test_unreadable_records_flag_partial_coverage(self):
        self.fixture("message.json")
        with mock.patch.object(Path, "open", side_effect=PermissionError):
            logs = q.read_logs(self.project.parent, q.Config(), 0)
        self.assertEqual(logs.unreadable, 1)
        self.assertIn(q.PARTIAL_WARNING, q.build_report(logs, q.Budget(25, "day", "usd"), "project", NOW)["attention"])


class CalendarAndPricingTests(unittest.TestCase):
    def test_calendar_arithmetic_and_rfc3339(self):
        self.assertEqual(q.days(1970, 1, 1), 0)
        for day in range(-20000, 30000):
            self.assertEqual(q.days(*q.civil(day)), day)
        self.assertIsNone(q.parse_date("2025-02-29T00:00:00Z"))
        self.assertEqual(q.parse_date("2024-02-29T02:00:00+02:00"), q.parse_date("2024-02-29T00:00:00.123Z"))
        for bad in ("2024-01-01T00:00:00Zjunk", "2024-01-01T24:00:00Z", "2024-01-01T00:00:00+24:00", None):
            self.assertIsNone(q.parse_date(bad))
        stamp = q.parse_date("2024-02-29T12:00:00Z")
        start, end = q.window(stamp, "month")
        self.assertEqual(q.label(start), "2024-02-01")
        self.assertEqual((end - start) // 86400, 29)
        start, end = q.window(stamp, "week")
        self.assertEqual(q.label(start), "2024-02-26")
        self.assertEqual((end - start) // 86400, 7)
        self.assertEqual(q.window(stamp, "day")[1] - q.window(stamp, "day")[0], 86400)
        self.assertEqual(q.label(q.window(q.parse_date("2024-12-31T23:59:59Z"), "month")[1]), "2025-01-01")
        with self.assertRaises(ValueError):
            q.window(stamp, "year")

    def test_prices_aliases_dates_unknowns_and_overrides(self):
        config = q.Config()
        opus = q.price("claude-opus-4-6", config)
        self.assertEqual((opus.input, opus.output, opus.cache_read, opus.cache_write), (5, 25, 0.5, 6.25))
        self.assertEqual(opus.cache_write_1h, 10)
        self.assertEqual(q.price("claude-opus-5-5", config).cache_read, 0.2)
        self.assertEqual(q.price("claude-fable-5-1", config).cache_read, 0.25)
        self.assertEqual(q.price("claude-opus-5-5-20261001", config).input, 4)
        self.assertEqual(q.price("claude-sonnet-4-20250514", config).input, 3)
        for model in ("claude-opus-future", "claude-opus-5-5-unknown"):
            self.assertIsNone(q.price(model, config))
        config.prices["claude-opus-future"] = q.Price(1, 2, 3, 4, 5)
        self.assertEqual(q.price("claude-opus-future", config).input, 1)

    def test_fixture_cost_token_validation_and_cache_partition(self):
        record = json.loads((FIXTURES / "message.json").read_text())
        _, item = q.event(record, "p", "s", q.Config())
        self.assertEqual(item.tokens, 1800)
        self.assertAlmostEqual(item.usd, 0.00756)
        record["message"]["usage"]["cache_creation"] = {"ephemeral_1h_input_tokens": 400}
        self.assertAlmostEqual(q.event(record, "p", "s", q.Config())[1].usd, 0.00846)
        record["message"]["usage"]["cache_creation"]["ephemeral_1h_input_tokens"] = 401
        self.assertIsNone(q.event(record, "p", "s", q.Config()))
        record["message"]["usage"].pop("cache_creation")
        for invalid in (-1, True, 1.5, None, 2**64):
            record["message"]["usage"]["input_tokens"] = invalid
            self.assertIsNone(q.event(record, "p", "s", q.Config()))

    def test_burn_and_projection_boundaries(self):
        self.assertEqual(q.projection(10, 20, 86400, 86400), (10, 172800))
        self.assertEqual(q.projection(21, 20, 86400, 86400)[1], 86400)
        self.assertEqual(q.projection(0, 20, 86400, 86400), (None, None))
        self.assertEqual(q.projection(10, 20, 0, 0), (None, None))
        self.assertEqual(q.projection(5e-324, 25, 86400, NOW), (0.0, None))

    def test_retention_and_stale_price_boundaries(self):
        budget = q.Budget(25, "month", "usd")
        start, _ = q.window(NOW, budget.per)
        for oldest, expected in ((None, False), (start - 1, False), (start, False), (start + 1, True)):
            report = q.build_report(q.Logs(oldest_timestamp=oldest), budget, "project", NOW)
            self.assertEqual(q.RETENTION_WARNING in report["attention"], expected)
        checked = q.parse_date(q.PRICING_DATE + "T00:00:00Z")
        for age, expected in ((90 * 86400, False), (90 * 86400 + 1, True)):
            report = q.build_report(q.Logs(), budget, "project", checked + age)
            self.assertEqual(q.STALE_PRICING_WARNING in report["attention"], expected)

    def test_window_filter_grouping_top_sessions_and_text_json_parity(self):
        start, end = q.window(NOW, "day")
        events = [q.Event(start, "p", str(i), "m", i * 10, float(i)) for i in range(1, 7)]
        events += [q.Event(t, "excluded", "s", "m", 999, 999) for t in (start - 1, end, NOW + 1)]
        logs = q.Logs(events=events, oldest_timestamp=start)
        for by, expected in (("project", "p"), ("model", "m"), ("day", q.label(NOW))):
            report = q.build_report(logs, q.Budget(25, "day", "usd"), by, NOW)
            self.assertEqual(report["observed_spent"], 21)
            self.assertEqual(report["messages"], 6)
            self.assertEqual(report["breakdown"][0]["name"], expected)
            self.assertEqual(len(report["expensive_sessions"]), 5)
            self.assertEqual(report["expensive_sessions"][0]["name"], "p/6")
            self.assertIn("observed: 21.0000", q.text_report(report))
            self.assertEqual(json.loads(json.dumps(report)), report)


class CliAndPromptTests(unittest.TestCase):
    def test_flags_and_choices(self):
        self.assertEqual(q.parse_args("budget set 25 --per week --unit usd --json".split()).per, q.Period.WEEK)
        self.assertTrue(q.parse_args("report --by model --json".split()).json)
        for invalid in ("budget set NaN --per day", "budget set -1 --per day", "budget set 20",
                        "budget set 1 --per year", "budget set 1 --per day --unit invalid", "--per day",
                        "report --by invalid", "report --by session", "report", "--once", "--tui --json",
                        "budget set 1 --per day --per week", "report --by day --by model", "--unknown"):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                q.parse_args(invalid.split())

    def test_prompt_defaults_validation_and_cancellation(self):
        stderr = io.StringIO()
        with mock.patch.object(sys, "stdin", io.StringIO("25\n\n\n")), contextlib.redirect_stderr(stderr):
            budget = q.prompt_budget()
        self.assertEqual(budget.encode(), {"amount": 25, "per": "month", "unit": "usd"})
        self.assertIn("Personal budget amount", stderr.getvalue())
        for value in ("", "x\n", "25\nyear\n", "25\nmonth\ninvalid\n"):
            with mock.patch.object(sys, "stdin", io.StringIO(value)), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(ValueError):
                    q.prompt_budget()

    def test_budget_text_preserves_precision(self):
        report = q.build_report(q.Logs(), q.Budget(25.123456789, "day", "usd"), "project", NOW)
        self.assertIn("budget: 25.123456789 usd / day", q.text_report(report))

    def test_help_does_not_require_home(self):
        with mock.patch.dict(os.environ, {}, clear=True), contextlib.redirect_stdout(io.StringIO()) as out:
            q.run(["--help"])
        self.assertIn("python3 quota_lite.py", out.getvalue())


if __name__ == "__main__":
    unittest.main()
