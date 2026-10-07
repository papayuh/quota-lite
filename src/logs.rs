use crate::{
    config::{price, Config},
    date,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::Path,
    time::{Duration, UNIX_EPOCH},
};
#[derive(Clone, Debug)]
pub struct Event {
    pub time: i64,
    pub project: String,
    pub session: String,
    pub model: String,
    pub tokens: f64,
    pub usd: Option<f64>,
}
#[derive(Default)]
pub struct Logs {
    pub events: Vec<Event>,
    /// Earliest valid transcript timestamp, including files outside the report window.
    pub oldest_timestamp: Option<i64>,
    pub files: usize,
    pub skipped: usize,
    pub unreadable: usize,
}
fn number(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_u64).map(|n| n as f64)
}
fn billable(v: &Value) -> bool {
    v.get("type").and_then(Value::as_str) == Some("assistant")
        && v.pointer("/message/model")
            .and_then(Value::as_str)
            .is_some_and(|m| m.starts_with("claude-"))
        && v.pointer("/message/role")
            .and_then(Value::as_str)
            .is_none_or(|role| role == "assistant")
}
fn event(v: &Value, project: &str, session: &str, c: &Config) -> Option<(String, Event)> {
    if !billable(v) {
        return None;
    }
    let m = v.get("message")?;
    let id = m.get("id")?.as_str()?;
    let model = m.get("model")?.as_str()?;
    let u = m.get("usage")?;
    let input = number(u, "input_tokens")?;
    let output = number(u, "output_tokens")?;
    let read = if u.get("cache_read_input_tokens").is_some() {
        number(u, "cache_read_input_tokens")?
    } else {
        0.0
    };
    let write = if u.get("cache_creation_input_tokens").is_some() {
        number(u, "cache_creation_input_tokens")?
    } else {
        0.0
    };
    let time = date::parse(v.get("timestamp")?.as_str()?)?;
    let p = price(model, c);
    // Cache TTL counts are subdivisions of total cache creation, not extra tokens.
    let hour = match u.pointer("/cache_creation/ephemeral_1h_input_tokens") {
        Some(n) => n.as_u64()?,
        None => 0,
    };
    if hour as f64 > write {
        return None;
    }
    let usd = p.and_then(|p| {
        let hour_rate = if hour > 0 { p.cache_write_1h? } else { 0.0 };
        Some(
            (input * p.input
                + output * p.output
                + read * p.cache_read
                + (write - hour as f64) * p.cache_write
                + hour as f64 * hour_rate)
                / 1e6,
        )
    });
    let usd = usd.filter(|n| n.is_finite());
    Some((
        id.into(),
        Event {
            time,
            project: project.into(),
            session: session.into(),
            model: model.into(),
            tokens: input + output + read + write,
            usd,
        },
    ))
}
struct Scan<'a> {
    config: &'a Config,
    since: i64,
    logs: Logs,
    messages: BTreeMap<String, Event>,
}
fn scan_file(path: &Path, project: &str, session: &str, scan: &mut Scan) {
    let cutoff = UNIX_EPOCH + Duration::from_secs(scan.since.max(0) as u64);
    let active = !fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t < cutoff);
    let (c, logs, messages) = (scan.config, &mut scan.logs, &mut scan.messages);
    let Ok(file) = File::open(path) else {
        logs.unreadable += usize::from(active);
        return;
    };
    logs.files += usize::from(active);
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        // Bound each line without ever retaining a whole large transcript line.
        let mut oversized = false;
        loop {
            let Ok(buf) = reader.fill_buf() else {
                logs.unreadable += usize::from(active);
                return;
            };
            if buf.is_empty() {
                break;
            }
            let end = buf
                .iter()
                .position(|b| *b == b'\n')
                .map(|i| i + 1)
                .unwrap_or(buf.len());
            let newline = buf[end - 1] == b'\n';
            if bytes.len() + end <= 4 * 1024 * 1024 && !oversized {
                bytes.extend_from_slice(&buf[..end]);
            } else {
                oversized = true;
            }
            reader.consume(end);
            if newline {
                break;
            }
        }
        if oversized {
            logs.skipped += usize::from(active);
            continue;
        }
        if bytes.is_empty() {
            break;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            logs.skipped += usize::from(active);
            continue;
        };
        if let Some(time) = v.get("timestamp").and_then(Value::as_str).and_then(date::parse) {
            logs.oldest_timestamp = Some(logs.oldest_timestamp.map_or(time, |old| old.min(time)));
        }
        if !active {
            continue;
        }
        if let Some((id, e)) = event(&v, project, session, c) {
            // Streaming assistant snapshots repeat message.id; latest usage wins.
            messages.insert(id, e);
        } else if billable(&v) && v.pointer("/message/usage").is_some() {
            logs.skipped += 1;
        }
    }
}
fn entries(path: &Path, logs: &mut Logs) -> Vec<fs::DirEntry> {
    let mut list: Vec<_> = match fs::read_dir(path) {
        Ok(it) => it
            .filter_map(|e| match e {
                Ok(e) => Some(e),
                Err(_) => {
                    logs.unreadable += 1;
                    None
                }
            })
            .collect(),
        Err(_) => {
            logs.unreadable += 1;
            Vec::new()
        }
    };
    list.sort_by_key(|e| e.file_name());
    list
}
fn regular(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}
fn directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}
pub fn read(root: &Path, c: &Config, since: i64) -> Logs {
    let mut scan = Scan {
        config: c,
        since,
        logs: Logs::default(),
        messages: BTreeMap::new(),
    };
    if root.exists() {
        walk(root, &mut scan);
    }
    scan.logs.events = scan.messages.into_values().collect();
    scan.logs
}
fn walk(root: &Path, scan: &mut Scan) {
    if !directory(root) {
        scan.logs.unreadable += 1;
        return;
    }
    for project in entries(root, &mut scan.logs) {
        if !directory(&project.path()) {
            continue;
        }
        let name = project.file_name().to_string_lossy().into_owned();
        for entry in entries(&project.path(), &mut scan.logs) {
            let path = entry.path();
            if path.extension().is_some_and(|x| x == "jsonl") && regular(&path) {
                let session = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                scan_file(&path, &name, &session, scan);
            } else if directory(&path) {
                // Claude Code layout: projects/<project>/<session>/subagents/*.jsonl.
                let sub = path.join("subagents");
                if directory(&sub) {
                    for agent in entries(&sub, &mut scan.logs) {
                        let p = agent.path();
                        if p.extension().is_some_and(|x| x == "jsonl") && regular(&p) {
                            let session = format!(
                                "{}/{}",
                                entry.file_name().to_string_lossy(),
                                p.file_stem().unwrap_or_default().to_string_lossy()
                            );
                            scan_file(&p, &name, &session, scan);
                        }
                    }
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_price() {
        let v: Value =
            serde_json::from_str(include_str!("../tests/fixtures/message.json")).unwrap();
        let (_, e) = event(&v, "p", "s", &Config::default()).unwrap();
        assert_eq!(e.tokens, 1800.0);
        assert!((e.usd.unwrap() - 0.00756).abs() < 1e-10);
        let mut unknown = v.clone();
        unknown["message"]["model"] = Value::String("claude-new-model".into());
        assert!(event(&unknown, "p", "s", &Config::default())
            .unwrap()
            .1
            .usd
            .is_none());
        unknown["message"]["usage"]["input_tokens"] = Value::from(-1);
        assert!(event(&unknown, "p", "s", &Config::default()).is_none());
        let mut hour = v.clone();
        hour["message"]["usage"]["cache_creation"] =
            serde_json::json!({"ephemeral_1h_input_tokens":400});
        assert!(
            (event(&hour, "p", "s", &Config::default())
                .unwrap()
                .1
                .usd
                .unwrap()
                - 0.00846)
                .abs()
                < 1e-10
        );
        let mut c = Config::default();
        c.prices.insert(
            "claude-sonnet-4-20250514".into(),
            crate::config::Price {
                input: 3.0,
                output: 15.0,
                cache_read: 0.3,
                cache_write: 3.75,
                cache_write_1h: Some(6.0),
            },
        );
        assert!((event(&hour, "p", "s", &c).unwrap().1.usd.unwrap() - 0.00846).abs() < 1e-10);
        let mut invalid = v;
        invalid["message"]["usage"]["cache_read_input_tokens"] = Value::from(-1);
        assert!(event(&invalid, "p", "s", &c).is_none());
    }
}
