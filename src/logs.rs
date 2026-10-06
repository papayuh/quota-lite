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
    pub files: usize,
    pub skipped: usize,
    pub unreadable: usize,
}
fn number(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_u64).map(|n| n as f64)
}
fn event(v: &Value, project: &str, session: &str, c: &Config) -> Option<(String, Event)> {
    if v.get("type")?.as_str()? != "assistant" {
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
    // One-hour cache creation requires explicit premium pricing; do not silently
    // price it at the five-minute rate unless the user supplied an override.
    let hour = match u.pointer("/cache_creation/ephemeral_1h_input_tokens") {
        Some(n) => n.as_u64()?,
        None => 0,
    };
    let usd = if hour > 0 && !c.prices.contains_key(model) {
        None
    } else {
        p.map(|p| {
            (input * p.input + output * p.output + read * p.cache_read + write * p.cache_write)
                / 1e6
        })
    };
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
fn scan_file(path: &Path, project: &str, session: &str, c: &Config, logs: &mut Logs) {
    let Ok(file) = File::open(path) else {
        logs.unreadable += 1;
        return;
    };
    logs.files += 1;
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::new();
    let mut messages: BTreeMap<String, Event> = BTreeMap::new();
    loop {
        bytes.clear();
        // Bound each line without ever retaining a whole large transcript line.
        let mut oversized = false;
        loop {
            let Ok(buf) = reader.fill_buf() else {
                logs.unreadable += 1;
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
            logs.skipped += 1;
            continue;
        }
        if bytes.is_empty() {
            break;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            logs.skipped += 1;
            continue;
        };
        if let Some((id, e)) = event(&v, project, session, c) {
            // Streaming assistant snapshots repeat message.id; latest usage wins.
            messages.insert(id, e);
        } else if v.get("type").and_then(Value::as_str) == Some("assistant")
            && v.pointer("/message/usage").is_some()
        {
            logs.skipped += 1;
        }
    }
    logs.events.extend(messages.into_values());
}
fn entries(path: &Path, logs: &mut Logs) -> Vec<fs::DirEntry> {
    match fs::read_dir(path) {
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
    }
}
fn regular(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}
fn directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}
pub fn read(root: &Path, c: &Config) -> Logs {
    let mut logs = Logs::default();
    if !root.exists() {
        return logs;
    }
    if !directory(root) {
        logs.unreadable += 1;
        return logs;
    }
    for project in entries(root, &mut logs) {
        if !directory(&project.path()) {
            continue;
        }
        let name = project.file_name().to_string_lossy().into_owned();
        for entry in entries(&project.path(), &mut logs) {
            let path = entry.path();
            if path.extension().is_some_and(|x| x == "jsonl") && regular(&path) {
                let session = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                scan_file(&path, &name, &session, c, &mut logs);
            } else if directory(&path) {
                // Claude Code layout: projects/<project>/<session>/subagents/*.jsonl.
                let sub = path.join("subagents");
                if directory(&sub) {
                    for agent in entries(&sub, &mut logs) {
                        let p = agent.path();
                        if p.extension().is_some_and(|x| x == "jsonl") && regular(&p) {
                            let session = format!(
                                "{}/{}",
                                entry.file_name().to_string_lossy(),
                                p.file_stem().unwrap_or_default().to_string_lossy()
                            );
                            scan_file(&p, &name, &session, c, &mut logs);
                        }
                    }
                }
            }
        }
    }
    logs
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
        unknown["message"]["model"] = Value::String("new-model".into());
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
        assert!(event(&hour, "p", "s", &Config::default())
            .unwrap()
            .1
            .usd
            .is_none());
        let mut c = Config::default();
        c.prices.insert(
            "claude-sonnet-4-20250514".into(),
            crate::config::Price {
                input: 3.0,
                output: 15.0,
                cache_read: 0.3,
                cache_write: 6.0,
            },
        );
        assert!((event(&hour, "p", "s", &c).unwrap().1.usd.unwrap() - 0.00846).abs() < 1e-10);
        let mut invalid = v;
        invalid["message"]["usage"]["cache_read_input_tokens"] = Value::from(-1);
        assert!(event(&invalid, "p", "s", &c).is_none());
    }
}
