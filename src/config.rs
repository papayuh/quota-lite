use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Budget {
    pub amount: f64,
    pub per: String,
    pub unit: String,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    pub budget: Option<Budget>,
    #[serde(default)]
    pub prices: BTreeMap<String, Price>,
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(b) = &self.budget {
            if !b.amount.is_finite()
                || b.amount <= 0.0
                || !["day", "week", "month"].contains(&b.per.as_str())
                || !["usd", "tokens", "requests"].contains(&b.unit.as_str())
            {
                return Err(
                    "invalid budget: positive amount, day|week|month, usd|tokens|requests required"
                        .into(),
                );
            }
        }
        for p in self.prices.values() {
            if [p.input, p.output, p.cache_read, p.cache_write]
                .iter()
                .any(|n| !n.is_finite() || *n < 0.0)
            {
                return Err("prices must be finite nonnegative USD per million tokens".into());
            }
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let meta = fs::symlink_metadata(path).map_err(|_| "cannot inspect config")?;
        if !meta.is_file() || meta.len() > 1024 * 1024 {
            return Err("config must be a regular file under 1 MiB".into());
        }
        let c: Self = serde_json::from_slice(&fs::read(path).map_err(|_| "cannot read config")?)
            .map_err(|_| "invalid config JSON")?;
        c.validate()?;
        Ok(c)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).map_err(|_| "cannot create config directory")?;
        }
        // Refuse to replace a symlink. Config contains no secrets.
        if fs::symlink_metadata(path).is_ok_and(|m| !m.is_file()) {
            return Err("config must be a regular file".into());
        }
        fs::write(
            path,
            serde_json::to_vec_pretty(self).map_err(|_| "cannot encode config")?,
        )
        .map_err(|_| "cannot write config".into())
    }
}
pub fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set".into())
}
pub fn path() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(home()?.join(".config"));
    Ok(base.join("quota-lite/config.json"))
}
// Public Anthropic list prices, checked 2026-10-06 (legacy 3.x snapshot 2025-05-22).
// USD per million tokens;
// cache writes use the five-minute rate. Exact known ids only, never guess new models.
pub fn price(model: &str, c: &Config) -> Option<Price> {
    if let Some(p) = c.prices.get(model) {
        return Some(p.clone());
    }
    let (i, o, read_multiplier) = match model {
        "claude-opus-5-5" => (4.0, 20.0, 0.05),
        "claude-sonnet-5-5" | "claude-sonnet-5" => (2.0, 10.0, 0.1),
        "claude-fable-5-1" | "claude-mythos-5-1" => (10.0, 50.0, 0.025),
        "claude-fable-5" | "claude-mythos-5" => (10.0, 50.0, 0.1),
        "claude-opus-5"
        | "claude-opus-4-8"
        | "claude-opus-4-7"
        | "claude-opus-4-6"
        | "claude-opus-4-5"
        | "claude-opus-4-5-20251101" => (5.0, 25.0, 0.1),
        "claude-haiku-4-5" | "claude-haiku-4-5-20251001" => (1.0, 5.0, 0.1),
        "claude-sonnet-4-6"
        | "claude-sonnet-4-5"
        | "claude-sonnet-4-5-20250929"
        | "claude-sonnet-4-20250514"
        | "claude-sonnet-4-0"
        | "claude-3-7-sonnet-20250219"
        | "claude-3-5-sonnet-20241022"
        | "claude-3-5-sonnet-20240620" => (3.0, 15.0, 0.1),
        "claude-opus-4-1"
        | "claude-opus-4-1-20250805"
        | "claude-opus-4-20250514"
        | "claude-opus-4-0"
        | "claude-3-opus-20240229" => (15.0, 75.0, 0.1),
        "claude-3-5-haiku-20241022" => (0.8, 4.0, 0.1),
        "claude-3-haiku-20240307" => (0.25, 1.25, 0.1),
        _ => return None,
    };
    Some(Price {
        input: i,
        output: o,
        cache_read: i * read_multiplier,
        cache_write: i * 1.25,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dated_prices() {
        let c = Config::default();
        let opus = price("claude-opus-4-6", &c).unwrap();
        assert_eq!(
            (opus.input, opus.output, opus.cache_read, opus.cache_write),
            (5.0, 25.0, 0.5, 6.25)
        );
        assert_eq!(price("claude-opus-5-5", &c).unwrap().cache_read, 0.2);
        assert_eq!(price("claude-fable-5-1", &c).unwrap().cache_read, 0.25);
        assert!(price("claude-opus-future", &c).is_none());
    }
}
