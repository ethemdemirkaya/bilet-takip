use chrono::{DateTime, Datelike, FixedOffset, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SourceId {
    Bubilet,
    Biletinial,
    Biletix,
}

impl SourceId {
    pub fn name(self) -> &'static str {
        match self {
            SourceId::Bubilet => "Bubilet",
            SourceId::Biletinial => "Biletinial",
            SourceId::Biletix => "Biletix",
        }
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Bir sitede satılan tek bir etkinlik/seans.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub source: SourceId,
    /// Kaynak içinde benzersiz kimlik (slug, etkinlik kodu, slug@tarih...)
    pub id: String,
    pub title: String,
    pub venue: String,
    pub date: Option<DateTime<FixedOffset>>,
    pub category: String,
    pub url: String,
    /// Kategori adı -> fiyat (kuruş)
    pub tiers: BTreeMap<String, i64>,
    pub sold_out: bool,
    /// Kategori adı -> sitenin indirimden önceki (üstü çizili) fiyatı, kuruş. Sadece indirimli kategoriler.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub list_prices: BTreeMap<String, i64>,
}

impl Event {
    pub fn key(&self) -> String {
        format!("{}:{}", self.source.name().to_lowercase(), self.id)
    }

    pub fn min_price(&self) -> Option<i64> {
        self.tiers.values().copied().min()
    }

    /// Sitede indirimli görünen kategoriler: (kategori, eski fiyat, indirimli fiyat), ucuzdan pahalıya.
    pub fn discounts(&self) -> Vec<(String, i64, i64)> {
        if self.sold_out {
            return Vec::new();
        }
        let mut v: Vec<(String, i64, i64)> = self
            .tiers
            .iter()
            .filter_map(|(tier, &now)| self.list_prices.get(tier).filter(|l| **l > now).map(|&l| (tier.clone(), l, now)))
            .collect();
        v.sort_by_key(|(tier, _, now)| (*now, tier.clone()));
        v
    }
}

/// Türkiye saati (2016'dan beri sabit UTC+3).
pub fn tr_offset() -> FixedOffset {
    FixedOffset::east_opt(3 * 3600).unwrap()
}

/// "₺1.350,00", "1.350,00", "1350" gibi metinleri kuruşa çevirir.
pub fn parse_tl(s: &str) -> Option<i64> {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_digit() || *c == ',' || *c == '.').collect();
    if cleaned.is_empty() {
        return None;
    }
    let (whole, frac) = match cleaned.rsplit_once(',') {
        Some((w, f)) => (w.replace('.', ""), f.to_string()),
        None => (cleaned.replace('.', ""), String::new()),
    };
    let whole: i64 = whole.parse().ok()?;
    let frac: i64 = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().ok()? * 10,
        _ => frac[..2].parse().ok()?,
    };
    Some(whole * 100 + frac)
}

/// TL cinsinden ondalıklı sayıyı kuruşa çevirir.
pub fn tl_to_kurus(tl: f64) -> i64 {
    (tl * 100.0).round() as i64
}

/// Kuruşu "1.450 ₺" / "1.450,50 ₺" biçiminde yazar.
pub fn fmt_tl(kurus: i64) -> String {
    let whole = kurus / 100;
    let frac = kurus % 100;
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(c);
    }
    if frac == 0 {
        format!("{grouped} ₺")
    } else {
        format!("{grouped},{frac:02} ₺")
    }
}

const MONTHS: [&str; 12] = [
    "Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim", "Kasım", "Aralık",
];
const WEEKDAYS: [&str; 7] = ["Pazartesi", "Salı", "Çarşamba", "Perşembe", "Cuma", "Cumartesi", "Pazar"];

/// "11 Ekim Cumartesi 21:00"
pub fn fmt_date(d: &DateTime<FixedOffset>) -> String {
    let d = d.with_timezone(&tr_offset());
    format!(
        "{} {} {} {:02}:{:02}",
        d.day(),
        MONTHS[d.month0() as usize],
        WEEKDAYS[d.weekday().num_days_from_monday() as usize],
        d.hour(),
        d.minute()
    )
}

/// "11 Eki"
pub fn fmt_date_short(d: &DateTime<FixedOffset>) -> String {
    let d = d.with_timezone(&tr_offset());
    let m: String = MONTHS[d.month0() as usize].chars().take(3).collect();
    format!("{} {}", d.day(), m)
}

/// Türkçe karakterleri sadeleştirip küçük harfe çevirir: "İçimizdeki Şeytan" -> "icimizdeki seytan"
pub fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'ç' | 'Ç' => 'c',
            'ğ' | 'Ğ' => 'g',
            'ı' | 'I' | 'İ' | 'i' => 'i',
            'ö' | 'Ö' => 'o',
            'ş' | 'Ş' => 's',
            'ü' | 'Ü' => 'u',
            'â' | 'Â' => 'a',
            'î' | 'Î' => 'i',
            'û' | 'Û' => 'u',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}

/// Başlıktan kaba kategori tahmini (kaynak kategori vermiyorsa).
pub fn guess_category(title: &str) -> String {
    let t = fold(title);
    if t.contains("stand") {
        "Stand-up".into()
    } else if t.contains("konser") || t.contains("concert") || t.contains("live") {
        "Konser".into()
    } else if t.contains("tiyatro") || t.contains("oyun") {
        "Tiyatro".into()
    } else {
        "Etkinlik".into()
    }
}

/// Telegram HTML modu için kaçış.
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tl_parsing() {
        assert_eq!(parse_tl("₺1.350,00"), Some(135_000));
        assert_eq!(parse_tl("1.450,5"), Some(145_050));
        assert_eq!(parse_tl("850"), Some(85_000));
        assert_eq!(parse_tl("₺"), None);
    }

    #[test]
    fn tl_formatting() {
        assert_eq!(fmt_tl(135_000), "1.350 ₺");
        assert_eq!(fmt_tl(37_500), "375 ₺");
        assert_eq!(fmt_tl(145_050), "1.450,50 ₺");
        assert_eq!(fmt_tl(1_234_567_800), "12.345.678 ₺");
    }

    #[test]
    fn folding() {
        assert_eq!(fold("İçimizdeki Şeytan"), "icimizdeki seytan");
        assert_eq!(fold("KAYSERİ"), "kayseri");
    }
}
