mod config;
mod date;
mod logs;
mod report;
mod types;
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use types::{By, Period, Unit};
const HELP: &str = concat!(
    "quota-lite: offline personal usage budgets (estimates, not vendor quotas)\n\n",
    "quota-lite budget set <amount> --per day|week|month [--unit usd|tokens|requests] [--json]\n",
    "quota-lite [--json]\n",
    "quota-lite report --by project|model|day [--json]\n",
    "quota-lite --tui [--once]\n\n",
    "UTC calendar periods: day, Monday-based week, calendar month.\n",
    "Config: $XDG_CONFIG_HOME/quota-lite/config.json, %APPDATA% on Windows, ",
    "or ~/.config/quota-lite/config.json\n",
    "Logs: $CLAUDE_CONFIG_DIR/projects or ~/.claude/projects (home: HOME, else USERPROFILE)\n",
    "Copilot usage is unknown: local premium-request accounting is not standardized.\n",
    "--tui redraws every 30 seconds; Ctrl-C quits. --once draws one frame.\n",
);
#[derive(Debug)]
struct Args {
    amount: Option<f64>,
    per: Period,
    unit: Unit,
    by: By,
    json: bool,
    tui: bool,
    once: bool,
    help: bool,
}
fn args(values: Vec<String>) -> Result<Args, String> {
    let mut a = Args {
        amount: None,
        per: Period::Month,
        unit: Unit::Usd,
        by: By::Project,
        json: false,
        tui: false,
        once: false,
        help: false,
    };
    let mut pos = 0;
    let mut set = false;
    let mut report = false;
    if values.first().is_some_and(|v| v == "budget") {
        if values.get(1).map(String::as_str) != Some("set") {
            return Err("expected budget set <amount> --per day|week|month".into());
        }
        a.amount = Some(
            values
                .get(2)
                .ok_or("missing amount")?
                .parse()
                .map_err(|_| "invalid amount")?,
        );
        set = true;
        pos = 3;
    } else if values.first().is_some_and(|v| v == "report") {
        report = true;
        pos = 1;
    }
    let mut per = false;
    let mut unit = false;
    let mut by = false;
    while pos < values.len() {
        let key = &values[pos];
        pos += 1;
        match key.as_str() {
            "--json" => a.json = true,
            "--tui" => a.tui = true,
            "--once" => a.once = true,
            "--help" | "-h" => a.help = true,
            "--per" | "--unit" | "--by" => {
                let value = values.get(pos).ok_or("missing flag value")?.clone();
                pos += 1;
                match key.as_str() {
                    "--per" => {
                        if per {
                            return Err("duplicate --per".into());
                        }
                        a.per = value.parse()?;
                        per = true;
                    }
                    "--unit" => {
                        if unit {
                            return Err("duplicate --unit".into());
                        }
                        a.unit = value.parse()?;
                        unit = true;
                    }
                    _ => {
                        if by {
                            return Err("duplicate --by".into());
                        }
                        a.by = value.parse()?;
                        by = true;
                    }
                }
            }
            _ => return Err("unknown command or flag; use --help".into()),
        }
    }
    if set && !per {
        return Err("budget set requires --per".into());
    }
    if (!set && (per || unit))
        || (!report && by)
        || (report && !by)
        || (a.tui && (a.json || set || report))
        || (a.once && !a.tui)
    {
        return Err("incompatible flags; use --help".into());
    }
    if set {
        config::Config {
            budget: Some(config::Budget {
                amount: a.amount.unwrap(),
                per: a.per,
                unit: a.unit,
            }),
            ..Default::default()
        }
        .validate()?;
    }
    Ok(a)
}
fn prompt() -> Result<config::Budget, String> {
    fn ask(label: &str, default: &str) -> Result<String, String> {
        eprint!("{label}");
        io::stderr().flush().map_err(|_| "cannot write prompt")?;
        let mut line = String::new();
        if io::stdin()
            .read_line(&mut line)
            .map_err(|_| "cannot read prompt")?
            == 0
        {
            return Err("budget prompt cancelled".into());
        }
        let s = line.trim();
        Ok(if s.is_empty() {
            default.into()
        } else {
            s.into()
        })
    }
    let amount = ask("Personal budget amount: ", "")?
        .parse()
        .map_err(|_| "invalid amount")?;
    let per = ask("Period day|week|month [month]: ", "month")?.parse()?;
    let unit = ask("Unit usd|tokens|requests [usd]: ", "usd")?.parse()?;
    Ok(config::Budget { amount, per, unit })
}
fn run() -> Result<(), String> {
    let a = args(std::env::args().skip(1).collect())?;
    if a.help {
        print!("{HELP}");
        return Ok(());
    }
    if a.tui && !io::stdout().is_terminal() && !a.once {
        return Err("--tui requires a terminal (or --once)".into());
    }
    let path = config::path()?;
    let mut c = config::Config::load(&path)?;
    if let Some(amount) = a.amount {
        c.budget = Some(config::Budget {
            amount,
            per: a.per,
            unit: a.unit,
        });
        c.save(&path)?;
        if a.json {
            println!(
                "{}",
                serde_json::to_string(&c).map_err(|_| "cannot encode config")?
            );
        } else {
            println!("Budget saved.");
        }
        return Ok(());
    }
    if c.budget.is_none() {
        if !io::stdin().is_terminal() {
            return Err(
                "no budget configured; run: quota-lite budget set <amount> --per day|week|month"
                    .into(),
            );
        }
        c.budget = Some(prompt()?);
        c.save(&path)?;
    }
    let b = c.budget.as_ref().ok_or("no budget")?;
    let root = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(p) => PathBuf::from(p),
        None => config::home()?.join(".claude"),
    }
    .join("projects");
    loop {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock predates 1970")?
            .as_secs() as i64;
        let r = report::build(
            logs::read(&root, &c, date::window(now, b.per).0),
            b,
            a.by,
            now,
        );
        if a.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&r).map_err(|_| "cannot encode report")?
            );
        } else {
            if a.tui && io::stdout().is_terminal() {
                print!("\x1b[2J\x1b[H");
            }
            print!("{}", report::text(&r));
        }
        io::stdout().flush().map_err(|_| "cannot write report")?;
        if !a.tui || a.once {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("quota-lite: {e}");
        std::process::exit(2);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parse(s: &str) -> Result<Args, String> {
        args(s.split_whitespace().map(String::from).collect())
    }
    #[test]
    fn flags() {
        assert!(parse("budget set 25 --per week --unit usd --json").is_ok());
        for s in [
            "budget set NaN --per day",
            "budget set -1 --per day",
            "budget set 20",
            "budget set 1 --per year",
            "budget set 1 --per day --unit invalid",
            "report --by session",
            "--per day",
            "report --by invalid",
            "report",
            "--once",
            "--tui --json",
        ] {
            assert!(parse(s).is_err(), "{s}");
        }
        assert!(parse("report --by model --json").is_ok());
    }
}
