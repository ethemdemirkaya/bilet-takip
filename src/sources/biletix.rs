//! Biletix: Solr arama servisi şehirdeki etkinlikleri verir, fiyatlar
//! `getPriceByProfiles/{etkinlikKodu}/001` uç noktasından (kuruş cinsinden) alınır.

use crate::config::Config;
use crate::http::Http;
use crate::model::{fold, Event, SourceId};
use anyhow::{Context, Result};
use chrono::DateTime;
use serde_json::Value;
use std::collections::BTreeMap;

const API: &str = "https://www.biletix.com/wbtxapi/api/v1/bxcached/event";

pub fn fetch(http: &Http, cfg: &Config) -> Result<Vec<Event>> {
    let url = format!(
        "https://www.biletix.com/solr/tr/select/?start=0&rows=300&q=*:*&fq=city:%22{}%22&fq=end%3A%5BNOW%20TO%20*%5D&wt=json",
        cfg.city
    );
    let solr = http.get_json(&url)?;
    let mut events = parse_solr(&solr)?;
    for ev in &mut events {
        http.pause();
        match http.get_json(&format!("{API}/getPriceByProfiles/{}/001/INTERNET/tr", ev.id)) {
            Ok(v) => ev.tiers = parse_prices(&v),
            Err(e) => eprintln!("  Biletix fiyat alınamadı ({}): {e:#}", ev.id),
        }
    }
    Ok(events)
}

pub fn parse_solr(v: &Value) -> Result<Vec<Event>> {
    let docs = v["response"]["docs"].as_array().context("Biletix: Solr yanıtında docs yok")?;
    Ok(docs
        .iter()
        .filter(|d| d["type"] == "event")
        .filter_map(|d| {
            let id = d["id"].as_str()?.to_string();
            let title = d["sname"].as_str()?.trim().to_string();
            let status = d["status"].as_str().unwrap_or("");
            Some(Event {
                source: SourceId::Biletix,
                url: format!("https://www.biletix.com/etkinlik/{id}/TURKIYE/tr"),
                venue: d["svenue"].as_str().unwrap_or("").trim().to_string(),
                date: d["start"].as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()),
                category: category(d["category"].as_str().unwrap_or(""), d["subcategory"].as_str().unwrap_or(""), &title),
                sold_out: status.to_lowercase().contains("sold"),
                list_price: None,
                tiers: BTreeMap::new(),
                id,
                title,
            })
        })
        .collect())
}

fn category(cat: &str, sub: &str, title: &str) -> String {
    let sub = fold(sub);
    if sub.contains("stand") || fold(title).contains("stand") {
        return "Stand-up".into();
    }
    match cat {
        "MUSIC" => "Konser",
        "ART" => "Tiyatro",
        "FAMILY" => "Çocuk/Aile",
        "SPORT" => "Spor",
        _ => "Etkinlik",
    }
    .into()
}

/// `{"data": {"<profilId>": [{"description": "VIP", "minPrice": 425000}, ...]}}`
/// Aynı kategori birden fazla profilde varsa en düşüğü alınır.
pub fn parse_prices(v: &Value) -> BTreeMap<String, i64> {
    let mut tiers: BTreeMap<String, i64> = BTreeMap::new();
    let Some(profiles) = v["data"].as_object() else { return tiers };
    for cats in profiles.values().filter_map(Value::as_array) {
        for c in cats {
            let (Some(name), Some(price)) = (c["description"].as_str(), c["minPrice"].as_i64()) else { continue };
            if price <= 0 {
                continue;
            }
            let e = tiers.entry(name.trim().to_string()).or_insert(price);
            *e = (*e).min(price);
        }
    }
    tiers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_solr_fixture() {
        let v: Value = serde_json::from_str(&std::fs::read_to_string("fixtures/biletix_solr_kayseri.json").unwrap()).unwrap();
        let events = parse_solr(&v).unwrap();
        assert!(events.len() >= 30);
        let karsu = events.iter().find(|e| e.id == "5MY27").unwrap();
        assert_eq!(karsu.title, "Karsu");
        assert_eq!(karsu.category, "Konser");
        assert_eq!(karsu.url, "https://www.biletix.com/etkinlik/5MY27/TURKIYE/tr");
    }

    #[test]
    fn parses_price_fixture() {
        let v: Value = serde_json::from_str(&std::fs::read_to_string("fixtures/biletix_prices_5MY27.json").unwrap()).unwrap();
        let tiers = parse_prices(&v);
        assert_eq!(tiers.get("VIP"), Some(&425_000));
        assert!(tiers.len() >= 5);
    }
}
