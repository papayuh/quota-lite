use crate::{
    config::Budget,
    date,
    logs::{Event, Logs},
};
use serde::Serialize;
use std::collections::BTreeMap;
#[derive(Serialize)]
pub struct Row {
    pub name: String,
    pub observed: f64,
    pub messages: usize,
    pub unpriced_messages: usize,
}
#[derive(Serialize)]
pub struct Report {
    pub estimate: bool,
    pub pricing_date: &'static str,
    pub unit: String,
    pub period: String,
    pub window_start: String,
    pub window_end_exclusive: String,
    pub budget: f64,
    pub observed_spent: Option<f64>,
    pub remaining: Option<f64>,
    pub burn_per_day: Option<f64>,
    pub projected_budget_hit: Option<String>,
    pub projection_within_window: Option<bool>,
    pub files_read: usize,
    pub messages: usize,
    pub unpriced_messages: usize,
    pub skipped_lines: usize,
    pub unreadable_paths: usize,
    pub breakdown: Vec<Row>,
    pub expensive_sessions: Vec<Row>,
    pub tips: Vec<String>,
    pub attention: Vec<String>,
}
fn value(e: &Event, unit: &str) -> f64 {
    if unit == "tokens" {
        e.tokens
    } else {
        e.usd.unwrap_or(0.0)
    }
}
fn group(events: &[&Event], unit: &str, by: &str) -> Vec<Row> {
    let mut rows: BTreeMap<String, Row> = BTreeMap::new();
    for e in events {
        let name = match by {
            "day" => date::label(e.time),
            "model" => e.model.clone(),
            "session" => format!("{}/{}", e.project, e.session),
            _ => e.project.clone(),
        };
        let row = rows.entry(name.clone()).or_insert(Row {
            name,
            observed: 0.0,
            messages: 0,
            unpriced_messages: 0,
        });
        row.observed += value(e, unit);
        row.messages += 1;
        row.unpriced_messages += usize::from(e.usd.is_none());
    }
    rows.into_values().collect()
}
pub fn projection(spent: f64, budget: f64, elapsed: f64, now: i64) -> (Option<f64>, Option<i64>) {
    if elapsed <= 0.0 || spent <= 0.0 {
        return (None, None);
    }
    let rate = spent / elapsed * 86400.0;
    let seconds = ((budget - spent).max(0.0) / rate * 86400.0).ceil();
    let hit = if seconds.is_finite() && seconds <= (i64::MAX - now) as f64 {
        Some(now + seconds as i64)
    } else {
        None
    };
    (Some(rate), hit)
}
pub fn build(logs: Logs, b: &Budget, by: &str, now: i64) -> Report {
    let (start, end) = date::window(now, &b.per);
    let events: Vec<_> = logs
        .events
        .iter()
        .filter(|e| e.time >= start && e.time < end && e.time <= now)
        .collect();
    let unknown = events.iter().filter(|e| e.usd.is_none()).count();
    let requests = b.unit == "requests";
    let spent = if requests {
        None
    } else {
        Some(events.iter().map(|e| value(e, &b.unit)).sum::<f64>())
    };
    let incomplete =
        requests || (b.unit == "usd" && unknown > 0) || logs.skipped > 0 || logs.unreadable > 0;
    let (burn, hit) = if incomplete {
        (None, None)
    } else {
        projection(spent.unwrap_or(0.0), b.amount, (now - start) as f64, now)
    };
    let mut sessions = if requests {
        vec![]
    } else {
        group(&events, &b.unit, "session")
    };
    sessions.sort_by(|a, b| b.observed.total_cmp(&a.observed).then(a.name.cmp(&b.name)));
    sessions.truncate(5);
    let mut tips = vec![];
    let models = group(&events, "usd", "model");
    let total = models.iter().map(|r| r.observed).sum::<f64>();
    if let Some(top) = models
        .iter()
        .max_by(|a, b| a.observed.total_cmp(&b.observed))
    {
        if total > 0.0 {
            tips.push(format!("{} accounts for {:.1}% of priced spend; consider a lower-priced model for routine tasks.",top.name,top.observed/total*100.0));
        }
    }
    let mut attention=vec!["Copilot premium requests, multipliers and costs are unmeasurable from supported local logs. No message-count proxy is used; requests budgets have unknown usage.".into(),"Local log coverage only; subscriptions, discounts, missing/deleted logs and vendor quotas are not measured.".into()];
    if logs.files == 0 {
        attention.push("No Claude JSONL session logs found; observed totals do not establish zero actual usage.".into());
    }
    if unknown > 0 {
        attention.push(format!("{unknown} messages lack a known price (or use one-hour cache writes); USD total is a lower bound. Add exact model overrides."));
    }
    if logs.skipped > 0 || logs.unreadable > 0 {
        attention.push("Some log records/paths were skipped; coverage is incomplete and projection is suppressed.".into());
    }
    Report {
        estimate: true,
        pricing_date: "2026-10-06",
        unit: b.unit.clone(),
        period: b.per.clone(),
        window_start: date::label(start),
        window_end_exclusive: date::label(end),
        budget: b.amount,
        observed_spent: spent,
        remaining: if incomplete {
            None
        } else {
            spent.map(|s| b.amount - s)
        },
        burn_per_day: burn,
        projected_budget_hit: hit.map(date::label),
        projection_within_window: hit.map(|h| h < end),
        files_read: logs.files,
        messages: events.len(),
        unpriced_messages: unknown,
        skipped_lines: logs.skipped,
        unreadable_paths: logs.unreadable,
        breakdown: if requests {
            vec![]
        } else {
            group(&events, &b.unit, by)
        },
        expensive_sessions: sessions,
        tips,
        attention,
    }
}
pub fn text(r: &Report) -> String {
    let n = |v: Option<f64>| v.map(|v| format!("{v:.4}")).unwrap_or("unknown".into());
    let mut s=format!("estimate: true (list prices {}, UTC calendar windows)\nbudget: {} {} / {}\nwindow: {} .. {} (exclusive)\nobserved: {} | remaining: {} | burn/day: {}\nprojectedHit: {} | withinWindow: {}\ncoverage: {} files, {} messages, {} unpriced, {} skipped, {} unreadable\n",r.pricing_date,r.budget,r.unit,r.period,r.window_start,r.window_end_exclusive,n(r.observed_spent),n(r.remaining),n(r.burn_per_day),r.projected_budget_hit.as_deref().unwrap_or("unknown"),r.projection_within_window.map(|v|v.to_string()).unwrap_or("unknown".into()),r.files_read,r.messages,r.unpriced_messages,r.skipped_lines,r.unreadable_paths);
    for (label, rows) in [
        ("breakdown", &r.breakdown),
        ("expensiveSessions", &r.expensive_sessions),
    ] {
        s.push_str(&format!(
            "{label}[{}]{{name,observed,messages,unpriced}}:\n",
            rows.len()
        ));
        for row in rows {
            s.push_str(&format!(
                "  {},{:.4},{},{}\n",
                serde_json::to_string(&row.name).unwrap_or_default(),
                row.observed,
                row.messages,
                row.unpriced_messages
            ));
        }
    }
    for tip in &r.tips {
        s.push_str(&format!(
            "tip: {}\n",
            serde_json::to_string(tip).unwrap_or_default()
        ));
    }
    for warning in &r.attention {
        s.push_str(&format!("attention: {warning}\n"));
    }
    s
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn burn() {
        assert_eq!(
            projection(10.0, 20.0, 86400.0, 86400),
            (Some(10.0), Some(172800))
        );
        assert_eq!(projection(21.0, 20.0, 86400.0, 86400).1, Some(86400));
        assert_eq!(projection(0.0, 20.0, 86400.0, 86400), (None, None));
        assert_eq!(projection(10.0, 20.0, 0.0, 0), (None, None));
    }
}
