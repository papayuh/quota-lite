use crate::{
    config::Budget,
    date,
    logs::{Event, Logs},
    types::{By, Period, Unit},
};
use serde::Serialize;
use std::collections::BTreeMap;
const PRICING_DATE: &str = "2026-10-06";
const RETENTION_WARNING: &str = concat!(
    "Report window starts before the oldest retained transcript. ",
    "Totals and burn rate are lower bounds; remaining and projection may be optimistic. ",
    "Claude Code cleanupPeriodDays defaults to 30 days and may be set lower by admins; ",
    "missing history may also reflect a new installation.",
);
const STALE_PRICING_WARNING: &str = concat!(
    "Built-in price table was checked more than 90 days ago; ",
    "estimates may use outdated rates. Check current prices and configure overrides.",
);
const COPILOT_WARNING: &str = concat!(
    "Copilot premium requests, multipliers and costs are unmeasurable from supported local logs. ",
    "No message-count proxy is used; requests budgets have unknown usage.",
);
const COVERAGE_WARNING: &str = concat!(
    "Local log coverage only; subscriptions, discounts, ",
    "missing/deleted logs and vendor quotas are not measured.",
);
const NO_LOGS_WARNING: &str = concat!(
    "No Claude JSONL session logs modified this period; ",
    "observed totals do not establish zero actual usage.",
);
const UNPRICED_WARNING: &str = concat!(
    "messages lack a known price; USD total is a lower bound. ",
    "Set prices[model] in config, including cache_write_1h for one-hour cache overrides.",
);
const PARTIAL_WARNING: &str = concat!(
    "Some log records/paths modified this period were skipped; ",
    "remaining, burn and projection are partial-coverage estimates.",
);
const MODEL_TIP: &str = "consider a lower-priced model for routine tasks.";
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
    pub unit: Unit,
    pub period: Period,
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
    pub unpriced_models: Vec<Row>,
    pub expensive_sessions: Vec<Row>,
    pub tips: Vec<String>,
    pub attention: Vec<String>,
}
fn value(e: &Event, unit: Unit) -> f64 {
    match unit {
        Unit::Tokens => e.tokens,
        Unit::Usd => e.usd.unwrap_or(0.0),
        Unit::Requests => unreachable!("requests usage is unmeasurable"),
    }
}
// None selects internal session grouping, never exposed as a CLI choice.
fn group(events: &[&Event], unit: Unit, by: Option<By>) -> Vec<Row> {
    let mut rows: BTreeMap<String, Row> = BTreeMap::new();
    for e in events {
        let name = match by {
            Some(By::Day) => date::label(e.time),
            Some(By::Model) => e.model.clone(),
            None => format!("{}/{}", e.project, e.session),
            Some(By::Project) => e.project.clone(),
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
pub fn build(logs: Logs, b: &Budget, by: By, now: i64) -> Report {
    let (start, end) = date::window(now, b.per);
    let events: Vec<_> = logs
        .events
        .iter()
        .filter(|e| e.time >= start && e.time < end && e.time <= now)
        .collect();
    let unknown = events.iter().filter(|e| e.usd.is_none()).count();
    let requests = b.unit == Unit::Requests;
    let spent = if requests {
        None
    } else {
        Some(events.iter().fold(0.0, |sum, e| sum + value(e, b.unit)))
    };
    let incomplete = requests || (b.unit == Unit::Usd && unknown > 0);
    let (burn, hit) = if incomplete {
        (None, None)
    } else {
        projection(spent.unwrap_or(0.0), b.amount, (now - start) as f64, now)
    };
    let mut sessions = if requests {
        vec![]
    } else {
        group(&events, b.unit, None)
    };
    sessions.sort_by(|a, b| b.observed.total_cmp(&a.observed).then(a.name.cmp(&b.name)));
    sessions.truncate(5);
    let mut tips = vec![];
    let models = group(&events, Unit::Usd, Some(By::Model));
    let total = models.iter().map(|r| r.observed).sum::<f64>();
    if let Some(top) = models
        .iter()
        .max_by(|a, b| a.observed.total_cmp(&b.observed))
    {
        if total > 0.0 {
            tips.push(format!(
                "{} accounts for {:.1}% of priced spend; {MODEL_TIP}",
                top.name,
                top.observed / total * 100.0,
            ));
        }
    }
    let mut attention = vec![COPILOT_WARNING.into(), COVERAGE_WARNING.into()];
    if logs.oldest_timestamp.is_some_and(|oldest| start < oldest) {
        attention.push(RETENTION_WARNING.into());
    }
    let checked = date::parse(&format!("{PRICING_DATE}T00:00:00Z")).expect("valid pricing date");
    if now - checked > 90 * 86400 {
        attention.push(STALE_PRICING_WARNING.into());
    }
    if logs.files == 0 {
        attention.push(NO_LOGS_WARNING.into());
    }
    if unknown > 0 {
        attention.push(format!("{unknown} {UNPRICED_WARNING}"));
    }
    if logs.skipped > 0 || logs.unreadable > 0 {
        attention.push(PARTIAL_WARNING.into());
    }
    Report {
        estimate: true,
        pricing_date: PRICING_DATE,
        unit: b.unit,
        period: b.per,
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
            group(&events, b.unit, Some(by))
        },
        unpriced_models: group(
            &events
                .iter()
                .copied()
                .filter(|e| e.usd.is_none())
                .collect::<Vec<_>>(),
            Unit::Usd,
            Some(By::Model),
        ),
        expensive_sessions: sessions,
        tips,
        attention,
    }
}
pub fn text(r: &Report) -> String {
    let n = |v: Option<f64>| v.map(|v| format!("{v:.4}")).unwrap_or("unknown".into());
    let mut s = format!(
        concat!(
            "estimate: true (list prices {}, UTC calendar windows)\n",
            "budget: {} {} / {}\n",
            "window: {} .. {} (exclusive)\n",
            "observed: {} | remaining: {} | burn/day: {}\n",
            "projectedHit: {} | withinWindow: {}\n",
            "coverage: {} files, {} messages, {} unpriced, {} skipped, {} unreadable\n",
        ),
        r.pricing_date,
        r.budget,
        r.unit,
        r.period,
        r.window_start,
        r.window_end_exclusive,
        n(r.observed_spent),
        n(r.remaining),
        n(r.burn_per_day),
        r.projected_budget_hit.as_deref().unwrap_or("unknown"),
        r.projection_within_window
            .map(|v| v.to_string())
            .unwrap_or("unknown".into()),
        r.files_read,
        r.messages,
        r.unpriced_messages,
        r.skipped_lines,
        r.unreadable_paths,
    );
    for (label, rows) in [
        ("breakdown", &r.breakdown),
        ("expensiveSessions", &r.expensive_sessions),
        ("unpricedModels", &r.unpriced_models),
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
        s.push_str(&format!(
            "attention: {}\n",
            serde_json::to_string(warning).unwrap_or_default()
        ));
    }
    s
}
#[cfg(test)]
mod tests {
    use super::*;
    fn empty_report(now: i64, oldest_timestamp: Option<i64>) -> Report {
        build(
            Logs {
                oldest_timestamp,
                ..Logs::default()
            },
            &Budget {
                amount: 25.0,
                per: Period::Month,
                unit: Unit::Usd,
            },
            By::Project,
            now,
        )
    }
    #[test]
    fn no_usage_is_positive_zero() {
        let r = empty_report(date::parse("2026-10-06T12:00:00Z").unwrap(), None);
        assert_eq!(r.observed_spent.unwrap().to_bits(), 0.0_f64.to_bits());
        assert!(!text(&r).contains("-0.0000"));
        assert_eq!(r.remaining, Some(25.0));
        assert!(r.burn_per_day.is_none());
    }
    #[test]
    fn retention_and_stale_price_boundaries() {
        let now = date::parse("2026-10-06T12:00:00Z").unwrap();
        let start = date::window(now, Period::Month).0;
        for (oldest, warn) in [
            (None, false),
            (Some(start - 1), false),
            (Some(start), false),
            (Some(start + 1), true),
        ] {
            assert_eq!(
                empty_report(now, oldest)
                    .attention
                    .iter()
                    .any(|s| s == RETENTION_WARNING),
                warn
            );
        }
        let checked = date::parse("2026-10-06T00:00:00Z").unwrap();
        for (age, warn) in [(90 * 86400, false), (90 * 86400 + 1, true)] {
            assert_eq!(
                empty_report(checked + age, None)
                    .attention
                    .iter()
                    .any(|s| s == STALE_PRICING_WARNING),
                warn
            );
        }
    }
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
