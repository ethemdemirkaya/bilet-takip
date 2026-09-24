use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub city: String,
    pub city_slug: String,
    pub thresholds: Thresholds,
    pub notify: Notify,
    pub sources: Sources,
    pub http: HttpConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Thresholds {
    pub min_drop_percent: f64,
    pub min_drop_tl: f64,
}

impl Thresholds {
    /// Eski fiyattan yeni fiyata düşüş bildirime değer mi?
    pub fn is_significant(&self, old: i64, new: i64) -> bool {
        if new >= old || old <= 0 {
            return false;
        }
        let drop = (old - new) as f64;
        drop * 100.0 >= self.min_drop_percent * old as f64 || drop >= self.min_drop_tl * 100.0
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Notify {
    pub price_drop: bool,
    pub new_event: bool,
    pub back_in_stock: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Sources {
    pub bubilet: bool,
    pub biletinial: bool,
    pub biletix: bool,
    pub biletinial_categories: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HttpConfig {
    pub delay_ms: u64,
    pub timeout_secs: u64,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("{} okunamadı", path.display()))?;
        toml::from_str(&text).with_context(|| format!("{} hatalı", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn significance() {
        let t = Thresholds { min_drop_percent: 5.0, min_drop_tl: 50.0 };
        assert!(t.is_significant(75_000, 37_500)); // %50
        assert!(t.is_significant(145_000, 140_000)); // 50 TL
        assert!(!t.is_significant(145_000, 142_000)); // 30 TL, %2
        assert!(!t.is_significant(100_000, 110_000)); // artış
    }
}
