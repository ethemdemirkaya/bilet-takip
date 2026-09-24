use crate::model::{Event, SourceId};
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Çalıştırmalar arasında saklanan durum (state/state.json).
/// Git'e commit'lendiği için her çalıştırmada değişen alan (ör. son çalışma zamanı) tutulmaz.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub events: BTreeMap<String, Tracked>,
    /// İlk taraması yapılmış kaynaklar (ilk taramada "yeni etkinlik" bildirimi gönderilmez)
    #[serde(default)]
    pub initialized_sources: BTreeSet<SourceId>,
    #[serde(default)]
    pub health: BTreeMap<SourceId, Health>,
    #[serde(default)]
    pub telegram_offset: i64,
    /// Telegram'dan /esik ile ayarlanan yüzde
    #[serde(default)]
    pub min_drop_percent_override: Option<f64>,
    /// GitHub, 60 gün hareketsiz repolarda zamanlanmış işleri durdurur; haftada bir güncellenir.
    #[serde(default)]
    pub keepalive: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tracked {
    pub event: Event,
    /// Kategori -> karşılaştırma fiyatı (son bildirilen ya da görülen en yüksek fiyat)
    pub reference: BTreeMap<String, i64>,
    pub first_seen: NaiveDate,
    pub last_seen: NaiveDate,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Health {
    pub consecutive_failures: u32,
    pub alerted: bool,
    pub last_count: usize,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl State {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).with_context(|| format!("{} bozuk", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)? + "\n")?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Tarihi geçen ya da 14 gündür hiç görülmeyen etkinlikleri siler.
    pub fn prune(&mut self, today: NaiveDate) {
        self.events.retain(|_, t| {
            let past = t.event.date.map(|d| d.date_naive() < today).unwrap_or(false);
            let stale = (today - t.last_seen).num_days() > 14;
            !past && !stale
        });
    }
}
