#[path = "../src/date.rs"]
mod date;
#[path = "../src/types.rs"]
mod types;
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let p = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "fixture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(p.join("claude/projects/synthetic-project")).unwrap();
        Self(p)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_quota-lite"))
            .args(args)
            .env("HOME", &self.0)
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("CLAUDE_CONFIG_DIR", self.0.join("claude"))
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    }
    fn logs(&self, data: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        // Midnight is always inside today's UTC period, even at midnight.
        let stamp = format!("{}T00:00:00Z", date::label(now));
        fs::write(
            self.0
                .join("claude/projects/synthetic-project/session.jsonl"),
            data.replace("2024-02-29T12:00:00Z", &stamp),
        )
        .unwrap();
    }
    fn json(&self, args: &[&str]) -> Value {
        let o = self.run(args);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let text = String::from_utf8(o.stdout).unwrap();
        assert!(!text.contains("SYNTHETIC_SECRET"));
        serde_json::from_str(&text).unwrap()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn e2e_dedup_unknown_prices_and_units() {
    let s = Sandbox::new();
    s.logs(include_str!("fixtures/session.jsonl"));
    assert!(!s.run(&["--json"]).status.success());
    assert!(s
        .run(&["budget", "set", "10", "--per", "day"])
        .status
        .success());
    let r = s.json(&["report", "--by", "model", "--json"]);
    assert_eq!(r["messages"], 2);
    assert_eq!(r["unpriced_messages"], 1);
    assert_eq!(r["unpriced_models"][0]["name"], "claude-future-model");
    assert!(String::from_utf8(s.run(&[]).stdout)
        .unwrap()
        .contains("claude-future-model"));
    assert!(r["attention"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().unwrap().contains("Set prices[model]")));
    assert!((r["observed_spent"].as_f64().unwrap() - 0.00756).abs() < 1e-10);
    assert!(r["remaining"].is_null());
    assert!(r["projected_budget_hit"].is_null());
    assert_eq!(r["breakdown"].as_array().unwrap().len(), 2);
    assert!(s
        .run(&["budget", "set", "10000", "--per", "week", "--unit", "tokens"])
        .status
        .success());
    assert_eq!(s.json(&["--json"])["observed_spent"], 2400.0);
    assert!(s
        .run(&["budget", "set", "100", "--per", "month", "--unit", "requests"])
        .status
        .success());
    let r = s.json(&["--json"]);
    assert!(r["observed_spent"].is_null());
    assert_eq!(r["breakdown"], serde_json::json!([]));
    assert!(s.run(&["--tui", "--once"]).status.success());
}
#[test]
fn no_usage_and_invalid_config_choices() {
    let s = Sandbox::new();
    for unit in ["usd", "tokens"] {
        assert!(s
            .run(&["budget", "set", "25", "--per", "month", "--unit", unit])
            .status
            .success());
        let r = s.json(&["--json"]);
        assert_eq!(
            r["observed_spent"].as_f64().unwrap().to_bits(),
            0.0_f64.to_bits()
        );
        assert_eq!(r["remaining"], 25.0);
        assert!(!String::from_utf8(s.run(&[]).stdout)
            .unwrap()
            .contains("-0.0000"));
    }
    let path = s.0.join("config/quota-lite/config.json");
    for budget in [
        serde_json::json!({"amount":25,"per":"year","unit":"usd"}),
        serde_json::json!({"amount":25,"per":"day","unit":"dollars"}),
    ] {
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"budget":budget})).unwrap(),
        )
        .unwrap();
        assert!(!s.run(&["--json"]).status.success());
    }
}
#[test]
fn overrides_malformed_and_subagents() {
    let s = Sandbox::new();
    assert!(s
        .run(&["budget", "set", "25", "--per", "month"])
        .status
        .success());
    let path = s.0.join("config/quota-lite/config.json");
    let mut c: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    c["prices"]["claude-sonnet-4-20250514"] =
        serde_json::json!({"input":1,"output":1,"cache_read":1,"cache_write":1});
    fs::write(&path, serde_json::to_vec(&c).unwrap()).unwrap();
    s.logs(include_str!("fixtures/message.json"));
    let r = s.json(&["--json"]);
    assert_eq!(r["observed_spent"], 0.0018);
    assert_eq!(r["unpriced_messages"], 0);
    let sub =
        s.0.join("claude/projects/synthetic-project/session/subagents");
    fs::create_dir_all(&sub).unwrap();
    let content = fs::read(s.0.join("claude/projects/synthetic-project/session.jsonl")).unwrap();
    fs::write(sub.join("agent.jsonl"), content).unwrap();
    let r = s.json(&["--json"]);
    assert_eq!(r["messages"], 1);
    assert_eq!(r["observed_spent"], 0.0018);
    s.logs("not json\n");
    let r = s.json(&["--json"]);
    assert_eq!(r["skipped_lines"], 1);
    assert_eq!(r["messages"], 1);
    assert!(r["burn_per_day"].as_f64().unwrap() > 0.0);
    assert!(r["remaining"].as_f64().is_some());
    assert!(r["projected_budget_hit"].is_string());
    assert!(r["attention"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().unwrap().contains("partial-coverage estimates")));
    fs::File::options()
        .write(true)
        .open(s.0.join("claude/projects/synthetic-project/session.jsonl"))
        .unwrap()
        .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(86400))
        .unwrap();
    let r = s.json(&["--json"]);
    assert_eq!(r["skipped_lines"], 0);
    assert_eq!(r["files_read"], 1);
    c["prices"]["claude-sonnet-4-20250514"]["input"] = Value::from(-1);
    fs::write(path, serde_json::to_vec(&c).unwrap()).unwrap();
    assert!(!s.run(&["--json"]).status.success());
}
#[test]
fn mixed_cache_and_non_api_entries() {
    let s = Sandbox::new();
    s.logs(include_str!("fixtures/mixed-cache.jsonl"));
    assert!(s
        .run(&["budget", "set", "10", "--per", "day"])
        .status
        .success());
    let r = s.json(&["--json"]);
    assert_eq!(r["messages"], 1);
    assert_eq!(r["unpriced_messages"], 0);
    assert_eq!(r["skipped_lines"], 0);
    assert!((r["observed_spent"].as_f64().unwrap() - 0.00801).abs() < 1e-10);
    // An old four-rate override cannot silently assign 5m rates to 1h writes.
    let path = s.0.join("config/quota-lite/config.json");
    let mut c: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    c["prices"]["claude-sonnet-4-20250514"] =
        serde_json::json!({"input":3,"output":15,"cache_read":0.3,"cache_write":3.75});
    fs::write(&path, serde_json::to_vec(&c).unwrap()).unwrap();
    assert_eq!(s.json(&["--json"])["unpriced_messages"], 1);
    c["prices"]["claude-sonnet-4-20250514"]["cache_write_1h"] = Value::from(6);
    fs::write(&path, serde_json::to_vec(&c).unwrap()).unwrap();
    assert_eq!(s.json(&["--json"])["unpriced_messages"], 0);
}
#[cfg(unix)]
#[test]
fn refuses_log_and_config_symlinks() {
    use std::os::unix::fs::symlink;
    let s = Sandbox::new();
    s.logs(include_str!("fixtures/message.json"));
    symlink(
        "session.jsonl",
        s.0.join("claude/projects/synthetic-project/copy.jsonl"),
    )
    .unwrap();
    assert!(s
        .run(&["budget", "set", "1", "--per", "day"])
        .status
        .success());
    assert_eq!(s.json(&["--json"])["files_read"], 1);
    let p = s.0.join("config/quota-lite/config.json");
    fs::rename(&p, p.with_extension("backup")).unwrap();
    symlink(p.with_extension("backup"), p).unwrap();
    assert!(!s
        .run(&["budget", "set", "2", "--per", "day"])
        .status
        .success());
}
#[test]
fn home_lookup_is_lazy_with_userprofile_fallback() {
    let s = Sandbox::new();
    let run = |args: &[&str], envs: &[(&str, PathBuf)]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_quota-lite"));
        cmd.args(args)
            .env_remove("HOME")
            .env_remove("USERPROFILE")
            .env_remove("APPDATA")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .stdin(std::process::Stdio::null());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    };
    let overrides = [
        ("XDG_CONFIG_HOME", s.0.join("config")),
        ("CLAUDE_CONFIG_DIR", s.0.join("claude")),
    ];
    assert!(run(&["budget", "set", "5", "--per", "day"], &overrides)
        .status
        .success());
    assert!(run(&["--json"], &overrides).status.success());
    assert!(!run(&["--json"], &[]).status.success());
    let profile = [("USERPROFILE", s.0.clone())];
    assert!(run(&["budget", "set", "5", "--per", "day"], &profile)
        .status
        .success());
    assert!(s.0.join(".config/quota-lite/config.json").is_file());
    fs::create_dir_all(s.0.join(".claude/projects/p")).unwrap();
    fs::write(s.0.join(".claude/projects/p/s.jsonl"), "").unwrap();
    let o = run(&["--json"], &profile);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let r: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(r["files_read"], 1);
}
